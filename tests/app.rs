mod common;

use common::*;

#[test]
fn appending_to_single_log_is_ok() {
    let mut cluster = TestCluster::new(1, 0, DEFAULT_CFG);
    cluster.tick_by(MAX_WAIT);
    let lead = cluster.get_by_id(0);
    assert_eq!(lead.log.entries.len(), 0);

    // append a few to log
    assert!(lead.client_request(50).is_ok());
    assert!(lead.client_request(100).is_ok());

    assert_eq!(lead.log.entries.len(), 2);
    assert_eq!(lead.log.committed_len, 2);
    assert_eq!(lead.log.applied_len, 2);
    assert_eq!(lead.log.app.get_state(), 150);
}

#[test]
fn appending_to_three_logs_is_ok() {
    let mut cluster = TestCluster::new(3, 0, DEFAULT_CFG);
    cluster.tick_by(MAX_WAIT);
    let mut lead = cluster.get_leader_mut().unwrap();
    assert_eq!(lead.log.entries.len(), 0);

    // append a few to log
    assert!(lead.client_request(50).is_ok());
    assert!(lead.client_request(100).is_ok());

    assert_eq!(lead.log.entries.len(), 2);
    assert_eq!(lead.log.committed_len, 0);
    assert_eq!(lead.log.applied_len, 0);
    assert_eq!(lead.log.app.get_state(), 0);

    // three ticks, one to propagate request another to propagate response
    // and another to propagate request from leader with updated commited_len
    cluster.tick_by(3);
    lead = cluster.get_leader_mut().unwrap();

    assert_eq!(lead.log.entries.len(), 2);
    assert_eq!(lead.log.committed_len, 2);
    assert_eq!(lead.log.applied_len, 2);
    assert_eq!(lead.log.app.get_state(), 150);

    // check follower state
    assert!(cluster.term_consensus());
    assert!(cluster.state_consensus());
}

#[test]
fn cannot_append_to_non_leader() {
    let mut cluster = TestCluster::new(3, 0, DEFAULT_CFG);
    cluster.kill(0);
    cluster.kill(1);
    cluster.tick_by(MAX_WAIT);
    let node = cluster.get_by_id(2);
    assert!(node.client_request(1).is_err());
}

#[test]
fn revive_old_leader_state_ok() {
    let mut cluster = TestCluster::new(3, 0, DEFAULT_CFG);
    cluster.tick_by(MAX_WAIT);
    let mut lead = cluster.get_leader_mut().unwrap();

    // append a few to log
    assert!(lead.client_request(1).is_ok());
    assert!(lead.client_request(2).is_ok());

    let lead_id = cluster.get_leader().unwrap().id;
    cluster.kill(lead_id);
    lead = cluster.get_leader_mut().unwrap();

    // ensure nothing propagates
    assert_eq!(lead.log.entries.len(), 2);
    assert_eq!(lead.log.committed_len, 0);
    assert_eq!(lead.log.applied_len, 0);
    assert_eq!(lead.log.app.get_state(), 0);
    cluster.tick_by(1);
    lead = cluster.get_leader_mut().unwrap();
    assert_eq!(lead.log.applied_len, 0);
    assert_eq!(lead.log.app.get_state(), 0);
    assert!(cluster.term_consensus());
    assert!(cluster.state_consensus());

    // settle into new consensus
    cluster.tick_by(MAX_WAIT);
    assert_eq!(cluster.num_leaders(), 2);

    // revive old leader, make sure leader is different (old one should be out of date)
    cluster.revive(lead_id);
    cluster.tick_by(MAX_WAIT);
    lead = cluster.get_leader_mut().unwrap();
    assert_eq!(lead.log.applied_len, 0);
    assert_eq!(lead.log.app.get_state(), 0);
    assert_eq!(cluster.num_leaders(), 1);
    assert_ne!(cluster.get_leader().unwrap().id, lead_id);
    assert!(cluster.term_consensus());
    assert!(cluster.state_consensus());
}

#[test]
fn leader_log_conflict_gets_resolved() {
    let mut cluster = TestCluster::new(3, 0, DEFAULT_CFG);
    cluster.tick_by(MAX_WAIT);
    let mut lead = cluster.get_leader_mut().unwrap();

    // add some entries to a log
    assert!(lead.client_request(1).is_ok());
    assert!(lead.client_request(2).is_ok());

    // kill leader before it is confirmed
    let lead_id = cluster.get_leader().unwrap().id;
    cluster.kill(lead_id);

    // re-settle
    cluster.tick_by(MAX_WAIT);
    assert_eq!(cluster.num_leaders(), 2);
    lead = cluster
        .peers
        .values_mut()
        .filter(|peer| peer.is_leader() && peer.id != lead_id)
        .nth(0)
        .unwrap();
    let new_lead_id = lead.id;

    // add entries to a log
    assert!(lead.client_request(3).is_ok());
    assert!(lead.client_request(4).is_ok());
    cluster.tick_by(MAX_WAIT);
    lead = cluster.get_by_id(new_lead_id);

    // ensure applied to state machine
    assert_eq!(lead.log.app.get_state(), 7);

    // revive old leader
    cluster.revive(lead_id);
    cluster.tick_by(MAX_WAIT);

    // ensure consensus with new state machine
    assert_eq!(cluster.get_by_id(lead_id).log.app.get_state(), 7);
    assert!(cluster.term_consensus());
    assert!(cluster.state_consensus());
}

#[test]
fn dead_node_catches_up_after_reviving() {
    let mut cluster = TestCluster::new(3, 0, DEFAULT_CFG);
    cluster.tick_by(MAX_WAIT);
    let mut lead = cluster.get_leader_mut().unwrap();

    // add some entries to a log
    assert!(lead.client_request(1).is_ok());
    assert!(lead.client_request(2).is_ok());
    cluster.tick_by(MAX_WAIT);

    // kill a node
    let follower_node_id = cluster
        .peers
        .values()
        .filter(|peer| !peer.is_leader())
        .nth(0)
        .unwrap()
        .id;
    cluster.kill(follower_node_id);
    lead = cluster.get_leader_mut().unwrap();

    // add more entries to a log
    assert!(lead.client_request(3).is_ok());
    assert!(lead.client_request(4).is_ok());
    cluster.tick_by(MAX_WAIT);

    // as one dead node is behind
    assert_eq!(cluster.get_leader().unwrap().log.app.get_state(), 10);
    assert!(!cluster.state_consensus());

    // revive node
    cluster.revive(follower_node_id);
    cluster.tick_by(MAX_WAIT);
    // ensure still consensus
    assert_eq!(cluster.get_leader().unwrap().log.app.get_state(), 10);
    assert!(cluster.state_consensus());
}
