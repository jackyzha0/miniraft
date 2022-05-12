use crate::server::Term;
use std::cmp::min;

/// Type alias for indexing into the [`Log`]
pub type LogIndex = usize;

/// A single log entry
#[derive(Clone)]
pub struct LogEntry<T> {
    /// What term it was submitted
    pub term: Term,

    /// Actual payload
    pub data: T,
}

/// A collection of LogEntries
pub struct Log<T, S> {
    pub entries: Vec<LogEntry<T>>,

    /// How much of the log has been considered committed.
    /// A log entry is considered 'safely replicated' or committed once it is replicated on a majority of servers.
    /// Only meaningful on servers which are leaders.
    /// Initialized to 0 as no entries are committed.
    /// Increases monotonically.
    pub committed_len: LogIndex,
    /// How much of the log has been applied to the state machine.
    /// Initialized to 0, increases monotonically.
    pub applied_len: LogIndex,

    /// State machine
    pub app: Box<dyn App<T, S>>,
}

impl<T, S> Log<T, S> {
    /// Instantiate a new empty event log
    pub fn new(app: Box<dyn App<T, S>>) -> Self {
        Log {
            entries: Vec::new(),
            committed_len: 0,
            applied_len: 0,
            app,
        }
    }

    /// Fetch the most recent term we have recorded in the log
    pub fn last_term(&self) -> Term {
        if self.entries.len() > 0 {
            self.entries.last().unwrap().term
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

    /// Append additional entries to the log.
    /// `prefix_idx` is what index caller expects entries to be inserted at,
    /// `leader_commit_idx` is the index of last log that leader has commited.
    pub fn append_entries(
        &mut self,
        prefix_idx: LogIndex,
        leader_commit_idx: LogIndex,
        mut entries: Vec<LogEntry<T>>,
    ) {
        // check to see if we need to truncate our existing log
        // this happens when we have conflicts between our log and leader's log
        // we roll back to last log entry that matches the leader
        if entries.len() > 0 && self.entries.len() > prefix_idx {
            let rollback_to = min(self.entries.len(), prefix_idx + entries.len()) - 1;
            let our_last_term = self
                .entries
                .get(rollback_to)
                .expect("rollback index was out of bounds")
                .term;
            let leader_last_term = entries
                .get(rollback_to - prefix_idx)
                .expect("leader first term index was out of bounds")
                .term;

            // truncate from start to rollback_to
            if our_last_term != leader_last_term {
                self.entries.truncate(rollback_to);
            }
        }

        // add all entries we don't have
        if prefix_idx + entries.len() > self.entries.len() {
            let new_entries_range = self.entries.len() - prefix_idx..;
            self.entries.extend(entries.drain(new_entries_range));
        }

        // leader has commited more messages than us, we can move forward and commit some of our messages
        if leader_commit_idx > self.committed_len {
            // apply each element log we haven't committed
            self.entries[self.committed_len..leader_commit_idx]
                .iter()
                .for_each(|entry| {
                    // apply each log entry to the state machine
                    self.app.transition_fn(entry);
                });

            // update commit index to reflect changes
            self.applied_len = leader_commit_idx;
            self.committed_len = leader_commit_idx;
        }
    }

    /// Deliver a single message from the message log to the application
    pub fn deliver_msg(&mut self) {
        let applied_idx = self.applied_len;
        self.app.transition_fn(
            self.entries
                .get(applied_idx)
                .expect("msg_idx of msg to be deliveres was out of bounds"),
        );
        self.applied_len += 1;
    }
}

pub trait App<T, S> {
    fn transition_fn(&mut self, entry: &LogEntry<T>);
    fn get_state(&self) -> S;
}
