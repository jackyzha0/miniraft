//! Quorum must be a strict majority. Computing it as ceil(n/2) admits two
//! disjoint quorums whenever the cluster size is even: a 2-2 partition of a
//! 4-node cluster then elects a leader on *both* sides in the same term, and
//! each side commits its own client commands — committed state diverges.

mod common;

use common::*;

fn partition_2_2(cluster: &mut TestCluster) {
    for a in [0, 1] {
        for b in [2, 3] {
            cluster.drop_between(a, b);
            cluster.drop_between(b, a);
        }
    }
}

#[test]
fn quorum_is_a_strict_majority() {
    for (n, want) in [(1, 1), (2, 2), (3, 2), (4, 3), (5, 3), (6, 4), (7, 4)] {
        let cluster = TestCluster::new(n, 0, DEFAULT_CFG);
        assert_eq!(cluster.peers[&0].quorum_size(), want, "cluster size {}", n);
    }
}

#[test]
fn four_node_split_brain_elects_no_leader() {
    let mut cluster = TestCluster::new(4, 0, DEFAULT_CFG);
    partition_2_2(&mut cluster);
    cluster.tick_by(4 * MAX_WAIT);

    // neither side of a 2-2 split can reach 3 votes, so there is no leader
    // and client requests are rejected everywhere instead of committing on
    // two divergent sides
    assert_eq!(cluster.num_leaders(), 0);
    for id in 0..4 {
        assert!(
            cluster.get_by_id(id).client_request(100).is_err(),
            "node {} accepted a client request while partitioned",
            id
        );
        assert_eq!(cluster.peers[&id].log.committed_len, 0);
    }
}

#[test]
fn four_node_cluster_recovers_after_partition_heals() {
    let mut cluster = TestCluster::new(4, 0, DEFAULT_CFG);
    partition_2_2(&mut cluster);
    cluster.tick_by(4 * MAX_WAIT);
    assert_eq!(cluster.num_leaders(), 0);

    cluster.drop_connections.clear();
    cluster.tick_by(MAX_WAIT);
    assert_eq!(cluster.num_leaders(), 1);

    // the cluster accepts and commits commands again
    cluster
        .get_leader_mut()
        .unwrap()
        .client_request(50)
        .unwrap();
    cluster.tick_by(4 * DEFAULT_CFG.heartbeat_interval + 2);
    assert!(cluster.term_consensus());
    assert!(cluster.state_consensus());
    assert_eq!(cluster.peers[&0].log.app.get_state(), 50);
}

#[test]
fn healthy_four_node_cluster_elects_single_leader() {
    let mut cluster = TestCluster::new(4, 0, DEFAULT_CFG);
    cluster.tick_by(MAX_WAIT);
    assert_eq!(cluster.num_leaders(), 1);
    cluster.tick_by(MAX_WAIT);
    assert_eq!(cluster.num_leaders(), 1);
    assert!(cluster.term_consensus());
}
