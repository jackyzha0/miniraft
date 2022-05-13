use std::collections::{BTreeMap, BTreeSet};

use miniraft::{
    debug::init_logger,
    log::{App, Log, LogEntry},
    server::{RaftConfig, RaftServer, ServerId},
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

pub fn setup() -> Log<u32, u32> {
    init_logger();
    let app = CountingApp { state: 0 };
    Log::new(0, Box::new(app))
}

pub struct TestCluster {
    transport: ReliableTransport<u32, u32>,
}

impl TestCluster {
    pub fn new(n: usize, seed: u64, config: RaftConfig) -> Self {
        let mut cluster = TestCluster {
            transport: ReliableTransport {
                peers: BTreeMap::new(),
                msg_queue: Vec::new(),
            },
        };
        let mut peers: BTreeSet<ServerId> = BTreeSet::new();
        (0..n).for_each(|id| {
            peers.insert(id);
        });

        for id in peers.clone() {
            let this_id = id.to_owned();
            let mut peers_without_this = peers.clone();
            peers_without_this.remove(&this_id);
            cluster.transport.peers.insert(
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
}
