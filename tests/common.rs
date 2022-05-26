#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;
use miniraft::{
    debug::init_logger,
    log::{App, Log, LogEntry},
    rpc::{SendableMessage, Target},
    server::{RaftConfig, RaftServer, ServerId},
};

use rand::{RngCore, SeedableRng};
use rand_chacha::ChaCha8Rng;

pub struct CountingApp {
    state: u32,
}

impl App<u32, u32> for CountingApp {
    fn transition_fn(&mut self, entry: &LogEntry<u32>) {
        self.state += entry.data;
    }
    fn get_state(&self) -> u32 {
        self.state
    }
}

pub fn setup_log() -> Log<u32, u32> {
    init_logger();
    let app = CountingApp { state: 0 };
    Log::new(0, Box::new(app))
}

pub struct TestCluster {
    pub msg_queue: Vec<(ServerId, SendableMessage<u32>)>,
    pub peers: BTreeMap<ServerId, RaftServer<u32, u32>>,
    pub drop_connections: BTreeSet<(Target, Target)>,
    pub down: BTreeSet<ServerId>,
}

/// Simulate a perfectly reliable transport medium that never drops packets
impl TestCluster {
    fn drop_between(&mut self, t1: Target, t2: Target) {
        self.drop_connections.insert((t1, t2));
    }

    fn isolate(&mut self, t: ServerId) {
        self.drop_connections
            .insert((Target::Single(t), Target::Broadcast));
        self.drop_connections
            .insert((Target::Broadcast, Target::Single(t)));
    }

    fn kill(&mut self, t: ServerId) {
        self.down.insert(t);
    }

    fn wrap_with_sender(
        from: ServerId,
        msgs: Vec<SendableMessage<u32>>,
    ) -> Vec<(ServerId, SendableMessage<u32>)> {
        msgs.into_iter().map(|msg| (from, msg)).collect()
    }

    fn tick(&mut self) -> Result<usize> {
        let old_msg_q_size = self.msg_queue.len();

        // iterate all peers which are alive and tick
        self.peers
            .values_mut()
            .filter(|peer| !self.down.contains(&peer.id))
            .for_each(|peer| {
                let new_msgs = Self::wrap_with_sender(peer.id, peer.tick());
                self.msg_queue.extend(new_msgs);
            });

        // send all things in msg queue
        let num_messages = self.msg_queue.len() - old_msg_q_size;
        let messages_to_send: Vec<(ServerId, SendableMessage<u32>)> =
            self.msg_queue.drain(..).collect();
        messages_to_send.iter().for_each(|(from, msg)| match msg {
            (t @ Target::Single(target), rpc) => {
                // get target peer, return an error if its not found
                let peer = self.peers.get_mut(&target).expect("peer not found");
                let new_msgs = Self::wrap_with_sender(peer.id, peer.receive_rpc(&rpc));
                self.msg_queue.extend(new_msgs);
            }
            (Target::Broadcast, rpc) => {
                // broadcast this message to all peers
                self.peers
                    .values_mut()
                    .map(|peer| Self::wrap_with_sender(peer.id, peer.receive_rpc(&rpc)))
                    .for_each(|new_msgs| self.msg_queue.extend(new_msgs));
            }
        });

        Ok(num_messages)
    }

    pub fn new(n: usize, seed: u64, config: RaftConfig) -> Self {
        init_logger();

        let mut cluster = TestCluster {
            peers: BTreeMap::new(),
            msg_queue: Vec::new(),
            drop_connections: BTreeSet::new(),
            down: BTreeSet::new(),
        };
        let mut peers: BTreeSet<ServerId> = BTreeSet::new();
        (0..n).for_each(|id| {
            peers.insert(id);
        });
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        for id in peers.clone() {
            let this_id = id.to_owned();
            let mut peers_without_this = peers.clone();
            peers_without_this.remove(&this_id);
            cluster.peers.insert(
                this_id,
                RaftServer::new(
                    this_id,
                    peers_without_this,
                    config.clone(),
                    Some(rng.next_u64()),
                    Box::new(CountingApp { state: 0 }),
                ),
            );
        }

        cluster
    }

    pub fn get_by_id(&self, id: ServerId) -> &RaftServer<u32, u32> {
        self.peers.get(&id).unwrap()
    }

    pub fn tick_by(&mut self, n: u32) -> &mut Self {
        (0..n).for_each(|_| {
            let _ = self.tick();
        });

        self
    }

    pub fn has_leader(&self) -> bool {
        self.peers.values().any(|peer| peer.is_leader())
    }

    pub fn has_candidate(&self) -> bool {
        self.peers.values().any(|peer| peer.is_candidate())
    }

    pub fn get_leader(&self) -> Option<&RaftServer<u32, u32>> {
        self.peers.values().filter(|peer| peer.is_leader()).last()
    }
}
