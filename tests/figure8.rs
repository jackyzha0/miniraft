//! Deterministic reproduction of the Raft paper's "figure 8" scenario
//! (§5.4.2): a leader must never commit an entry from a *previous* term by
//! counting replicas, because that entry can still be overwritten by a node
//! whose log has a higher last term — losing a committed entry and diverging
//! the state machines.
//!
//! The dance, on a 3-node cluster {l1, l2, f}:
//!   1. l1 leads term 1 and appends entry A, but is cut off before A
//!      replicates anywhere.
//!   2. l2 leads a later term and appends a conflicting entry B at the same
//!      index, but is cut off before B replicates anywhere.
//!   3. l1 comes back, wins an election (its log beats f's empty log), and
//!      replicates A to f. A now sits on a quorum {l1, f}. An implementation
//!      that counts replicas without checking terms commits and applies A.
//!   4. l2 comes back and wins an election (its last log term beats
//!      A's term on f), and overwrites A with B on f. If A was applied in
//!      step 3, the cluster has now applied two different commands at the
//!      same log position: state machine safety is violated.

mod common;

use common::*;
use miniraft::server::ServerId;

/// Leader in the highest term among servers that are actually running
fn alive_leader(cluster: &TestCluster) -> Option<ServerId> {
    cluster
        .peers
        .values()
        .filter(|p| p.is_leader() && !cluster.down.contains(&p.id))
        .max_by_key(|p| p.current_term)
        .map(|p| p.id)
}

/// Tick until `cond` holds, panicking after MAX_TICKS
fn wait_until(cluster: &mut TestCluster, what: &str, cond: impl Fn(&TestCluster) -> bool) {
    for _ in 0..MAX_TICKS {
        if cond(cluster) {
            return;
        }
        cluster.tick_by(1);
    }
    panic!("timed out after {} ticks waiting for: {}", MAX_TICKS, what);
}

#[test]
fn figure8_committed_entries_are_never_overwritten() {
    let mut cluster = TestCluster::new(3, 0, DEFAULT_CFG);

    // 1) elect the initial leader l1
    wait_until(&mut cluster, "initial leader", |c| alive_leader(c).is_some());
    let l1 = alive_leader(&cluster).unwrap();
    let others: Vec<ServerId> = cluster.peers.keys().copied().filter(|id| *id != l1).collect();

    // 2) cut l1 off, then hand it entry A: it lands in l1's log only
    for id in &others {
        cluster.kill(*id);
    }
    cluster.get_by_id(l1).client_request(100).unwrap();
    assert_eq!(cluster.peers[&l1].log.entries.len(), 1);
    assert_eq!(cluster.peers[&l1].log.committed_len, 0);

    // 3) park l1; the other two elect l2 in a later term
    cluster.kill(l1);
    for id in &others {
        cluster.revive(*id);
    }
    wait_until(&mut cluster, "second leader", |c| alive_leader(c).is_some());
    let l2 = alive_leader(&cluster).unwrap();
    let f = *others.iter().find(|id| **id != l2).unwrap();

    // 4) cut l2 off from f, then hand it entry B: it lands in l2's log only,
    //    conflicting with A at index 0
    cluster.drop_between(l2, f);
    cluster.drop_between(f, l2);
    cluster.get_by_id(l2).client_request(200).unwrap();
    assert_eq!(cluster.peers[&l2].log.entries.len(), 1);
    assert_eq!(cluster.peers[&f].log.entries.len(), 0);

    // 5) park l2, heal the network, revive l1. l1's log (last term 1) beats
    //    f's empty log, so l1 wins an election and replicates A to f. A is
    //    now on a quorum, but it is an entry from a previous term: committing
    //    it by counting replicas here is the bug this test exists to catch.
    cluster.kill(l2);
    cluster.drop_connections.clear();
    cluster.revive(l1);
    wait_until(&mut cluster, "entry A replicated to f", |c| {
        c.peers[&f].log.entries.len() == 1
    });
    cluster.tick_by(MAX_WAIT); // time enough to (unsafely) commit + apply A

    // 6) park l1; revive l2. l2's last log term beats f's, so it wins the
    //    election and legally overwrites A with B on f.
    cluster.kill(l1);
    cluster.revive(l2);
    wait_until(&mut cluster, "entry B overwrites A on f", |c| {
        c.peers[&f].log.entries.first().map(|e| e.data) == Some(200)
    });

    // give l2 a command in its *own* term so the log can commit, then let
    // the whole cluster settle
    cluster
        .get_by_id(l2)
        .client_request(300)
        .expect("l2 should still be leader");
    cluster.tick_by(MAX_WAIT);
    cluster.revive(l1);
    cluster.tick_by(4 * MAX_WAIT);

    // state machine safety: every node must have applied the same commands
    // in the same order. If A was committed in step 5, l1/f applied A(=100)
    // while l2 applied B(=200) at the same position and the states diverge.
    let states: Vec<(ServerId, u32)> = cluster
        .peers
        .values()
        .map(|p| (p.id, p.log.app.get_state()))
        .collect();
    assert!(
        cluster.state_consensus(),
        "state machines diverged after figure-8 dance: {:?}",
        states
    );
}
