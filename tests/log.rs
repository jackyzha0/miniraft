mod common;
use common::*;

use miniraft::log::LogEntry;

#[test]
fn last_term_and_index_of_empty() {
    let l = setup();
    assert_eq!(l.last_term(), 0);
    assert_eq!(l.last_idx(), 0);
    assert_eq!(l.app.get_state(), 0);
}

#[test]
fn last_term_and_index_of_non_empty() {
    let mut l = setup();
    l.entries.push(LogEntry { term: 0, data: 1 });
    l.entries.push(LogEntry { term: 0, data: 2 });
    assert_eq!(l.last_term(), 0);
    assert_eq!(l.last_idx(), 1);

    l.entries.push(LogEntry { term: 1, data: 3 });
    assert_eq!(l.last_term(), 1);
    assert_eq!(l.last_idx(), 2);
}

#[test]
fn apply_to_state() {
    let mut l = setup();
    l.entries.push(LogEntry { term: 0, data: 5 });
    l.deliver_msg();
    assert_eq!(l.applied_len, 1);
    assert_eq!(l.app.get_state(), 5);

    l.entries.push(LogEntry { term: 1, data: 3 });
    l.entries.push(LogEntry { term: 3, data: 2 });
    assert_eq!(l.applied_len, 1);
    assert_eq!(l.app.get_state(), 5);
    assert_eq!(l.last_term(), 3);
    assert_eq!(l.last_idx(), 2);

    l.deliver_msg();
    l.deliver_msg();
    assert_eq!(l.applied_len - 1, l.last_idx());
    assert_eq!(l.app.get_state(), 5 + 3 + 2);
}

#[test]
fn append_entries_empty_no_commit() {
    let mut l = setup();
    let entries = vec![
        LogEntry { term: 0, data: 1 },
        LogEntry { term: 0, data: 2 },
        LogEntry { term: 1, data: 3 },
    ];
    l.append_entries(0, 0, entries);
    assert_eq!(l.applied_len, 0);
    assert_eq!(l.app.get_state(), 0);
    assert_eq!(l.last_idx(), 2);
    assert_eq!(l.last_term(), 1);
}

#[test]
fn append_entries_empty_commit() {
    let mut l = setup();
    let entries = vec![
        LogEntry { term: 0, data: 1 },
        LogEntry { term: 0, data: 2 },
        LogEntry { term: 1, data: 3 },
    ];
    l.append_entries(0, 2, entries);
    assert_eq!(l.applied_len, 2);
    assert_eq!(l.app.get_state(), 3);
    assert_eq!(l.last_idx(), 2);
    assert_eq!(l.last_term(), 1);
}

#[test]
fn append_entries_non_empty_no_conflict() {
    let mut l = setup();
    l.append_entries(
        0,
        2,
        vec![LogEntry { term: 0, data: 1 }, LogEntry { term: 0, data: 2 }],
    );

    let entries = vec![
        LogEntry { term: 0, data: 3 },
        LogEntry { term: 0, data: 4 },
        LogEntry { term: 1, data: 5 },
    ];
    l.append_entries(2, 2, entries);
    assert_eq!(l.applied_len, 2);
    assert_eq!(l.app.get_state(), 3);
    assert_eq!(l.last_idx(), 4);
    assert_eq!(l.last_term(), 1);
}

#[test]
fn append_entries_leader_force_overwrite() {
    let mut l = setup();
    l.append_entries(
        0,
        0,
        vec![
            LogEntry { term: 0, data: 1 },
            LogEntry { term: 1, data: 2 },
            LogEntry { term: 1, data: 3 },
        ],
    );

    let entries = vec![LogEntry { term: 1, data: 2 }, LogEntry { term: 2, data: 5 }];
    l.append_entries(0, 2, entries);
    assert_eq!(l.applied_len, 2);
    assert_eq!(l.app.get_state(), 7);
    assert_eq!(l.last_idx(), 1);
    assert_eq!(l.last_term(), 2);
}

#[test]
fn append_entries_non_empty_conflict_append() {
    let mut l = setup();
    l.append_entries(
        0,
        0,
        vec![
            LogEntry { term: 0, data: 1 },
            LogEntry { term: 1, data: 2 },
            LogEntry { term: 1, data: 3 },
        ],
    );

    let entries = vec![LogEntry { term: 1, data: 4 }, LogEntry { term: 2, data: 5 }];
    l.append_entries(1, 3, entries);
    assert_eq!(l.applied_len, 3);
    assert_eq!(l.app.get_state(), 10);
    assert_eq!(l.last_idx(), 2);
    assert_eq!(l.last_term(), 2);
}

#[test]
fn append_entries_idempotency() {
    let mut l = setup();
    l.append_entries(
        0,
        2,
        vec![LogEntry { term: 0, data: 1 }, LogEntry { term: 1, data: 2 }],
    );
    l.append_entries(
        0,
        2,
        vec![LogEntry { term: 0, data: 1 }, LogEntry { term: 1, data: 2 }],
    );
    assert_eq!(l.applied_len, 2);
    assert_eq!(l.app.get_state(), 3);
    assert_eq!(l.last_idx(), 1);
    assert_eq!(l.last_term(), 1);
}
