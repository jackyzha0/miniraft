mod common;

use common::*;
use miniraft::server::RaftConfig;

const DEFAULT_CFG: RaftConfig = RaftConfig {
    election_timeout: 10,
    election_timeout_jitter: 4,
    heartbeat_interval: 5,
};

const MAX_WAIT: u32 = DEFAULT_CFG.election_timeout + DEFAULT_CFG.election_timeout_jitter;
const MAX_TICKS: u32 = 1_000;

#[test]
fn trivial_case_one_server_remains_leader() {
    let mut cluster = TestCluster::new(1, 0, DEFAULT_CFG);
    assert!(cluster.has_n_leaders(0));
    cluster.tick_by(MAX_WAIT);
    assert!(!cluster.has_candidate());
    assert!(cluster.has_n_leaders(1));
    cluster.tick_by(MAX_WAIT);
    assert!(cluster.has_n_leaders(1));
    assert!(cluster.leader_term() == 1);
    assert!(cluster.term_consensus());
}

#[test]
fn three_servers_one_server_remains_leader() {
    let mut cluster = TestCluster::new(3, 0, DEFAULT_CFG);
    assert!(cluster.has_n_leaders(0));
    cluster.tick_by(MAX_WAIT);
    assert!(cluster.has_n_leaders(1));
}

#[test]
fn large_number_servers_one_leader_remains_leader() {
    let mut cluster = TestCluster::new(47, 0, DEFAULT_CFG);
    assert!(cluster.has_n_leaders(0));
    cluster.tick_by(MAX_WAIT);
    assert!(cluster.has_n_leaders(1));
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
    assert!(cluster.has_n_leaders(0));
    cluster.tick_by(MAX_TICKS);
    assert!(cluster.has_n_leaders(0));
}

#[test]
fn two_cluster_partition_has_two_leaders() {
    let mut cluster = TestCluster::new(2, 0, DEFAULT_CFG);
    cluster.drop_between(0, 1);
    cluster.tick_by(MAX_WAIT);
    assert!(cluster.has_n_leaders(2));
    assert!(!cluster.has_candidate());
}

#[test]
fn three_cluster_partition_has_one_leader() {
    let mut cluster = TestCluster::new(3, 0, DEFAULT_CFG);
    cluster.drop_between(0, 1);
    cluster.tick_by(MAX_WAIT);
    assert!(cluster.has_n_leaders(1));
    assert!(!cluster.has_candidate());
    assert!(cluster.leader_term() == 1);
    assert!(cluster.term_consensus());
}

// term mismatch between two candidates (still ok, term is just higher)
// term mismatch from leader (kill leader for a while then let it take over)

// degraded (take some nodes down) -> cluster of 3
#[test]
fn degraded_all_down_has_no_leader() {
    let mut cluster = TestCluster::new(3, 0, DEFAULT_CFG);
    cluster.kill(0);
    cluster.kill(1);
    cluster.kill(2);
    assert!(cluster.has_n_leaders(0));
    cluster.tick_by(MAX_TICKS);
    assert!(cluster.has_n_leaders(0));
}

// no leader when all down
// still leader when at least one alive
#[test]
fn degraded_one_alive() {
    let mut cluster = TestCluster::new(3, 0, DEFAULT_CFG);
    cluster.kill(0);
    cluster.kill(1);
    cluster.tick_by(MAX_WAIT);
    // cannot achieve quorum w 1/3
    assert!(cluster.has_n_leaders(0));
    cluster.revive(1);
    cluster.revive(0);
    cluster.tick_by(MAX_TICKS);
    assert!(cluster.has_n_leaders(1))
}
