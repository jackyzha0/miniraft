mod common;

use std::collections::BTreeMap;

use common::*;
use miniraft::server::{NodeReplicationState, RaftConfig, ServerId};

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
    assert_eq!(cluster.num_leaders(), 0);
    cluster.tick_by(MAX_WAIT);
    assert!(!cluster.has_candidate());
    assert_eq!(cluster.num_leaders(), 1);
    cluster.tick_by(MAX_WAIT);
    assert_eq!(cluster.num_leaders(), 1);
    assert!(cluster.leader_term() == 1);
    assert!(cluster.term_consensus());
}

#[test]
fn three_servers_one_server_remains_leader() {
    let mut cluster = TestCluster::new(3, 0, DEFAULT_CFG);
    assert_eq!(cluster.num_leaders(), 0);
    cluster.tick_by(MAX_WAIT);
    assert_eq!(cluster.num_leaders(), 1);
}

#[test]
fn large_number_servers_one_leader_remains_leader() {
    let mut cluster = TestCluster::new(47, 0, DEFAULT_CFG);
    assert_eq!(cluster.num_leaders(), 0);
    cluster.tick_by(MAX_WAIT);
    assert_eq!(cluster.num_leaders(), 1);
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
    assert_eq!(cluster.num_leaders(), 0);
    cluster.tick_by(MAX_TICKS);
    assert_eq!(cluster.num_leaders(), 0);
}

#[test]
fn two_cluster_partition_has_two_leaders() {
    let mut cluster = TestCluster::new(2, 0, DEFAULT_CFG);
    cluster.drop_between(0, 1);
    cluster.tick_by(MAX_WAIT);
    assert_eq!(cluster.num_leaders(), 2);
    assert!(!cluster.has_candidate());
}

#[test]
fn three_cluster_partition_has_one_leader() {
    let mut cluster = TestCluster::new(3, 0, DEFAULT_CFG);
    cluster.drop_between(0, 1);
    cluster.tick_by(MAX_WAIT);
    assert_eq!(cluster.num_leaders(), 1);
    assert!(!cluster.has_candidate());
    assert!(cluster.leader_term() == 1);
    assert!(cluster.term_consensus());
}

#[test]
fn candidate_mismatched_terms() {
    let mut cluster = TestCluster::new(3, 0, DEFAULT_CFG);
    cluster.get_by_id(0).current_term = 99;
    cluster.tick_by(MAX_WAIT);
    assert_eq!(cluster.num_leaders(), 1);
    assert!(cluster.term_consensus());
    assert_eq!(cluster.leader_term(), 100);
}

#[test]
fn demote_leader_after_outdated_term() {
    let mut cluster = TestCluster::new(3, 0, DEFAULT_CFG);
    let mut followers: BTreeMap<ServerId, NodeReplicationState> = BTreeMap::new();
    followers.insert(1, Default::default());
    followers.insert(2, Default::default());
    // propogate heartbeats
    cluster.get_by_id(0).promote_to_leader(followers);
    cluster.tick_by(MAX_WAIT);

    // kill leader and let someone else take over
    cluster.kill(0);
    cluster.tick_by(MAX_WAIT);
    assert_eq!(cluster.num_leaders(), 2);
    cluster.revive(0);
    cluster.tick_by(MAX_WAIT);
    assert_ne!(cluster.get_leader().unwrap().id, 0);
}

#[test]
fn degraded_all_down_has_no_leader() {
    let mut cluster = TestCluster::new(3, 0, DEFAULT_CFG);
    cluster.kill(0);
    cluster.kill(1);
    cluster.kill(2);
    assert_eq!(cluster.num_leaders(), 0);
    cluster.tick_by(MAX_TICKS);
    assert_eq!(cluster.num_leaders(), 0);
}

#[test]
fn two_down_of_three_does_not_achieve_quorum() {
    let mut cluster = TestCluster::new(3, 0, DEFAULT_CFG);
    cluster.kill(0);
    cluster.kill(1);
    cluster.tick_by(MAX_WAIT);
    // cannot achieve quorum w 1/3
    assert_eq!(cluster.num_leaders(), 0);
    cluster.revive(1);
    cluster.revive(0);
    cluster.tick_by(MAX_TICKS);
    assert_eq!(cluster.num_leaders(), 1);
}
