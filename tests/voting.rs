mod common;

use common::*;
use miniraft::server::RaftConfig;

const DEFAULT_CFG: RaftConfig = RaftConfig {
    election_timeout: 10,
    election_timeout_jitter: 0,
    heartbeat_interval: 5,
};

#[test]
fn trivial_case_one_server() {
    let mut cluster = TestCluster::new(1, 0, DEFAULT_CFG);
    assert!(!cluster.has_leader());
    cluster.tick_by(DEFAULT_CFG.election_timeout);
    assert!(cluster.has_candidate());
    assert!(!cluster.has_leader());
    cluster.tick_by(1);
    assert!(!cluster.has_candidate());
    assert!(cluster.has_leader());
}
