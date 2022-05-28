#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;
use miniraft::{
    debug::init_logger,
    log::{App, Log, LogEntry},
    rpc::{SendableMessage, Target},
    server::{RaftConfig, RaftServer, ServerId, Term},
};

use rand::{RngCore, SeedableRng};
use rand_chacha::ChaCha8Rng;

pub const DEFAULT_CFG: RaftConfig = RaftConfig {
    election_timeout: 10,
    election_timeout_jitter: 4,
    heartbeat_interval: 5,
};

pub const MAX_WAIT: u32 = DEFAULT_CFG.election_timeout + DEFAULT_CFG.election_timeout_jitter;
pub const MAX_TICKS: u32 = 1_000;

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
    pub drop_connections: BTreeSet<(ServerId, ServerId)>,
    pub down: BTreeSet<ServerId>,
}

/// Simulate a perfectly reliable transport medium that never drops packets
impl TestCluster {
    pub fn drop_between(&mut self, t1: ServerId, t2: ServerId) {
        self.drop_connections.insert((t1, t2));
    }
    pub fn kill(&mut self, t: ServerId) {
        self.down.insert(t);
    }

    pub fn revive(&mut self, t: ServerId) {
        self.down.remove(&t);
    }

    fn should_drop(&self, from: ServerId, to: ServerId) -> bool {
        self.down.contains(&to) || self.drop_connections.contains(&(from, to))
    }

    fn tick(&mut self) -> Result<usize> {
        let old_msg_q_size = self.msg_queue.len();

        // iterate all peers which are alive and tick
        self.peers
            .values_mut()
            .filter(|peer| !self.down.contains(&peer.id))
            .for_each(|peer| {
                let new_msgs = wrap_with_sender(peer.id, peer.tick());
                self.msg_queue.extend(new_msgs);
            });

        // send all things in msg queue
        let num_messages = self.msg_queue.len() - old_msg_q_size;
        let messages_to_send: Vec<(ServerId, SendableMessage<u32>)> =
            self.msg_queue.drain(..).collect();
        messages_to_send.iter().for_each(|(from, msg)| match msg {
            (Target::Single(to), rpc) => {
                if !self.should_drop(from.to_owned(), to.to_owned()) {
                    // get target peer, return an error if its not found
                    let peer = self.peers.get_mut(&to).expect("peer not found");
                    let new_msgs = wrap_with_sender(peer.id, peer.receive_rpc(&rpc));
                    self.msg_queue.extend(new_msgs);
                }
            }
            (Target::Broadcast, rpc) => {
                self.peers
                    .values_mut()
                    .filter(|peer| {
                        let to = peer.id;
                        let should_drop = self.down.contains(&to)
                            || self.drop_connections.contains(&(from.to_owned(), to));
                        !should_drop
                    })
                    .map(|peer| wrap_with_sender(peer.id, peer.receive_rpc(&rpc)))
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

    pub fn get_by_id(&mut self, id: ServerId) -> &mut RaftServer<u32, u32> {
        self.peers.get_mut(&id).unwrap()
    }

    pub fn tick_by(&mut self, n: u32) -> &mut Self {
        (0..n).for_each(|_| {
            let _ = self.tick();
        });

        self
    }

    pub fn num_leaders(&self) -> usize {
        self.peers
            .values()
            .filter(|peer| peer.is_leader())
            .collect::<Vec<_>>()
            .len()
    }

    pub fn has_candidate(&self) -> bool {
        self.peers.values().any(|peer| peer.is_candidate())
    }

    pub fn get_leader(&self) -> Option<&RaftServer<u32, u32>> {
        self.peers.values().filter(|peer| peer.is_leader()).last()
    }

    pub fn leader_term(&self) -> Term {
        self.get_leader().unwrap().current_term
    }

    pub fn term_consensus(&self) -> bool {
        let l_term = self.leader_term();
        self.peers
            .values()
            .filter(|peer| peer.current_term != l_term)
            .collect::<Vec<_>>()
            .len()
            == 0
    }
}

fn wrap_with_sender(
    from: ServerId,
    msgs: Vec<SendableMessage<u32>>,
) -> Vec<(ServerId, SendableMessage<u32>)> {
    msgs.into_iter().map(|msg| (from, msg)).collect()
}
