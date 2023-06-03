use crate::{
    debug::Logger,
    server::{ServerId, Term},
};
use std::{
    cmp::min,
    fmt::{self, Debug},
};

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
pub struct Log<T, S> {
    /// Log entries
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

    /// [`ServerId`] of our parent for pretty printing documentation
    pub parent_id: ServerId,
}

impl<T, S> Log<T, S>
where
    T: fmt::Debug,
{
    /// Instantiate a new empty event log
    pub fn new(parent_id: ServerId, app: Box<dyn App<T, S>>) -> Self {
        Log {
            entries: Vec::new(),
            committed_len: 0,
            applied_len: 0,
            app,
            parent_id,
        }
    }

    /// Fetch the most recent term we have recorded in the log
    pub fn last_term(&self) -> Term {
        self.entries.last().map(|x| x.term).unwrap_or(0)
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
    /// `leader_commit_len` is the index of last log that leader has commited.
    pub fn append_entries(
        &mut self,
        prefix_idx: LogIndex,
        leader_commit_len: LogIndex,
        mut entries: Vec<LogEntry<T>>,
    ) {
        Logger::append_entries_recv(&self, prefix_idx, leader_commit_len, &entries);
        // check to see if we need to truncate our existing log
        // this happens when we have conflicts between our log and leader's log
        if entries.len() > 0 && self.entries.len() > prefix_idx {
            // we pick the last log index we can compare between leader and follower
            // either the last entry in the follower's log or last entry in the
            // new logs, whichever comes first
            let rollback_to = min(self.entries.len(), prefix_idx + entries.len()) - 1;
            let our_last_term = self.entries.get(rollback_to).unwrap().term;
            let leader_last_term = entries.get(rollback_to - prefix_idx).unwrap().term;
            Logger::log_potential_conflict(&self, &entries, prefix_idx, rollback_to);

            // truncate from start to rollback_to
            if our_last_term != leader_last_term {
                self.entries.truncate(prefix_idx);
                Logger::log_term_conflict(&self);
            }
        }

        // add all entries we don't have
        if prefix_idx + entries.len() > self.entries.len() {
            let start = self.entries.len() - prefix_idx;
            let new_entries_range = start..;
            self.entries.extend(entries.drain(new_entries_range));
            Logger::log_append(&self, start);
        }

        // leader has commited more messages than us, we can move forward and commit some of our messages
        if leader_commit_len > self.committed_len {
            // apply each element we haven't committed
            self.entries[self.committed_len..leader_commit_len]
                .iter()
                .for_each(|entry| {
                    self.app.transition_fn(entry);
                });

            Logger::log_apply(&self, leader_commit_len);
            // update commit index to reflect changes
            self.applied_len = leader_commit_len;
            self.committed_len = leader_commit_len;
        }
    }

    /// Deliver a single message from the message log to the application
    pub fn deliver_msg(&mut self) {
        Logger::log_deliver_recv(&self);

        let applied_idx = self.applied_len;
        self.app.transition_fn(
            self.entries
                .get(applied_idx)
                .expect("msg_idx of msg to be delivered was out of bounds"),
        );
        self.applied_len += 1;
        Logger::log_deliver_apply(&self);
    }
}

/// Describes a state machine that is updated bassed off of a feed of [`LogEntry`]
pub trait App<T, S> {
    /// Function that mutates the application state depending on the newest log entry.
    /// Raft guarantees that if the transition function is called on a [`LogEntry`], it is
    /// considered applied (meaning it won't be re-run or removed).
    fn transition_fn(&mut self, entry: &LogEntry<T>);

    /// Return the current state of the application
    fn get_state(&self) -> S;
}
