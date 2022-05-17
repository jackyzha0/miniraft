#![allow(dead_code)]

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Debug,
};

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

pub struct TestCluster(ReliableTransport<u32, u32>);

impl TestCluster {
    pub fn new(n: usize, seed: u64, config: RaftConfig) -> Self {
        init_logger();

        let mut cluster = TestCluster(ReliableTransport {
            peers: BTreeMap::new(),
            msg_queue: Vec::new(),
        });
        let mut peers: BTreeSet<ServerId> = BTreeSet::new();
        (0..n).for_each(|id| {
            peers.insert(id);
        });
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        for id in peers.clone() {
            let this_id = id.to_owned();
            let mut peers_without_this = peers.clone();
            peers_without_this.remove(&this_id);
            cluster.0.peers.insert(
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
        self.0.peers.get(&id).unwrap()
    }

    pub fn tick_by(&mut self, n: u32) -> &mut Self {
        (0..n).for_each(|_| {
            let _ = self.0.tick();
        });

        self
    }

    pub fn has_leader(&self) -> bool {
        self.0.peers.values().any(|peer| peer.is_leader())
    }

    pub fn has_candidate(&self) -> bool {
        self.0.peers.values().any(|peer| peer.is_candidate())
    }

    pub fn get_leader(&self) -> Option<&RaftServer<u32, u32>> {
        self.0.peers.values().filter(|peer| peer.is_leader()).last()
    }
}

pub trait TransportMedium<T> {
    /// Returns how many messages were passed in that tick
    fn tick(&mut self) -> Result<usize>;
}

pub struct ReliableTransport<T, S> {
    pub msg_queue: Vec<SendableMessage<T>>,
    pub peers: BTreeMap<ServerId, RaftServer<T, S>>,
}

/// Simulate a perfectly reliable transport medium that never drops packets
impl<T, S> TransportMedium<T> for ReliableTransport<T, S>
where
    T: Clone + Debug,
{
    fn tick(&mut self) -> Result<usize> {
        let old_msg_q_size = self.msg_queue.len();

        // iterate all peers and tick
        self.peers.values_mut().for_each(|peer| {
            let new_msgs = peer.tick();
            self.msg_queue.extend(new_msgs);
        });

        // send all things in msg queue
        let num_messages = self.msg_queue.len() - old_msg_q_size;
        let messages_to_send: Vec<SendableMessage<T>> = self.msg_queue.drain(..).collect();
        messages_to_send.iter().for_each(|msg| match msg {
            (Target::Single(target), rpc) => {
                // get target peer, return an error if its not found
                let peer = self.peers.get_mut(&target).expect("peer not found");
                let new_msgs = peer.receive_rpc(&rpc);
                self.msg_queue.extend(new_msgs);
            }
            (Target::Broadcast, rpc) => {
                // broadcast this message to all peers
                self.peers
                    .values_mut()
                    .map(|peer| peer.receive_rpc(&rpc))
                    .for_each(|new_msgs| self.msg_queue.extend(new_msgs));
            }
        });

        Ok(num_messages)
    }
}
