use std::{
    collections::{BTreeMap, BTreeSet},
    slice::SliceIndex,
};

use miniraft::{
    debug::init_logger,
    log::{App, Log, LogEntry},
    server::{RaftConfig, RaftLeadershipState, RaftServer, ServerId},
    transport::ReliableTransport,
};

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
                    Some(seed),
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
        self.0.peers.values_mut().for_each(|peer| {
            peer.tick();
        });
        self
    }

    pub fn has_leader(&self) -> bool {
        self.0.peers.values().any(|peer| peer.is_leader())
    }

    pub fn has_candidate(&self) -> bool {
        self.0.peers.values().any(|peer| peer.is_candidate())
    }
}