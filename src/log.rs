use std::collections::VecDeque;

use crate::server::Term;

/// Type alias for indexing into the [`Log`]
pub type LogIndex = usize;

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
    /// A log entry is considered 'safely replicated' or committed once it is replicated on a majority of servers.
    /// Initialized to 0, increases monotonically.
    pub commit_idx: LogIndex,
    /// Index of highest log entry applied to state machine.
    /// Initialized to 0, increases monotonically.
    pub last_applied: LogIndex,
}

impl<T> Log<T> {
    /// Instantiate a new empty event log
    pub fn new() -> Self {
        Log {
            entries: VecDeque::new(),
            commit_idx: 0,
            last_applied: 0,
        }
    }

    /// Fetch the most recent term we have recorded in the log
    pub fn last_term(&self) -> Term {
        if self.entries.len() > 0 {
            self.entries.back().unwrap().term
        } else {
            0
        }
    }

    /// Get index of the last element
    pub fn last_idx(&self) -> LogIndex {
        if self.entries.len() > 0 {
            self.entries.len() - 1
        } else {
            0
        }
    }
}
