use std::collections::VecDeque;

use crate::server::Term;

/// Type alias for indexing into the [`Log`]
pub type LogIndex = u64;

/// A single log entry
#[derive(Clone, Debug)]
pub struct LogEntry<T> {
    /// What term it was submitted
    pub term: Term,

    /// Actual payload
    pub data: T,
}

/// A collection of LogEntries
#[derive(Debug)]
pub struct Log<T> {
    pub entries: VecDeque<LogEntry<T>>,

    /// Index of highest log entry known to be commited.
    /// Initialized to 0, increases monotonically.
    pub commit_idx: LogIndex,
    /// Index of highest log entry applied to state machine.
    /// Initialized to 0, increases monotonically.
    pub last_applied: LogIndex,
}

impl<T> Log<T> {
    pub fn new() -> Self {
        Log {
            entries: VecDeque::new(),
            commit_idx: 0,
            last_applied: 0,
        }
    }
}
