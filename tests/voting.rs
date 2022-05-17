mod common;

use common::*;
use miniraft::server::RaftConfig;

const DEFAULT_CFG: RaftConfig = RaftConfig {
    election_timeout: 10,
    election_timeout_jitter: 4,
    heartbeat_interval: 5,
};

const MAX_WAIT: u32 = DEFAULT_CFG.election_timeout + DEFAULT_CFG.election_timeout_jitter;
const MAX_TICKS: u32 = 5000;

#[test]
fn trivial_case_one_server_remains_leader() {
    let mut cluster = TestCluster::new(1, 0, DEFAULT_CFG);
    assert!(!cluster.has_leader());
    cluster.tick_by(MAX_WAIT);
    assert!(!cluster.has_candidate());
    assert!(cluster.has_leader());
    cluster.tick_by(MAX_WAIT);
    assert!(cluster.has_leader());
}

#[test]
fn three_servers_one_leader_remains_leader() {
    let mut cluster = TestCluster::new(3, 0, DEFAULT_CFG);
    assert!(!cluster.has_leader());
    cluster.tick_by(MAX_WAIT);
    assert!(cluster.has_leader());
}

#[test]
fn no_jitter_never_has_leader() {
    let mut cluster = TestCluster::new(
        3,
        0,
        RaftConfig {
            election_timeout_jitter: 0,
            ..DEFAULT_CFG
        },
    );
    assert!(!cluster.has_leader());
    cluster.tick_by(MAX_TICKS);
    assert!(!cluster.has_leader());
}
