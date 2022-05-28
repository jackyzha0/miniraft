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
