use crate::server::Term;
use std::cmp::min;

/// Type alias for indexing into the [`Log`]
pub type LogIndex = usize;

/// Hacking a trait alias for a closure that consumes log entries
pub trait LogConsumer<T>: FnMut(&LogEntry<T>) {}
impl<T, F> LogConsumer<T> for F where F: FnMut(&LogEntry<T>) {}

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

    /// Index of highest log entry known to be commited.
    /// A log entry is considered 'safely replicated' or committed once it is replicated on a majority of servers.
    /// Initialized to 0, increases monotonically.
    pub commit_idx: LogIndex,
    /// Index of highest log entry applied to state machine.
    /// Initialized to 0, increases monotonically.
    pub last_applied: LogIndex,

    app: Box<dyn App<T, S>>,
}

impl<T, S> Log<T, S> {
    /// Instantiate a new empty event log
    pub fn new(app: Box<dyn App<T, S>>) -> Self {
        Log {
            entries: Vec::new(),
            commit_idx: 0,
            last_applied: 0,
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
            let our_last_term = self.entries.get(rollback_to).unwrap().term;
            let leader_last_term = entries.get(rollback_to - prefix_idx).unwrap().term;

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
        if leader_commit_idx > self.commit_idx {
            // apply each element log we haven't committed
            self.entries[self.commit_idx..leader_commit_idx]
                .iter()
                .enumerate()
                .for_each(|(i, entry)| {
                    // apply each log entry to the state machine
                    self.app.transition_fn(entry);
                    self.last_applied = self.commit_idx + i;
                });

            self.commit_idx = leader_commit_idx;
        }
    }
}

pub trait App<T, S> {
    fn transition_fn(&mut self, entry: &LogEntry<T>);
    fn get_state(&self) -> S;
}

#[cfg(test)]
mod tests {
    use crate::log::*;

    pub struct CountingApp {
        state: u32,
    }

    impl App<u32, u32> for CountingApp {
        fn transition_fn(&mut self, entry: &LogEntry<u32>) {
            self.state += entry.data;
        }
        fn get_state(&self) -> u32 {
            self.state
        }
    }

    fn setup() -> CountingApp {
        CountingApp { state: 0 }
    }

    #[test]
    fn last_term_and_index_of_empty() {
        let app = setup();
        let l: Log<u32, u32> = Log::new(Box::new(app));
        assert_eq!(l.last_term(), 0);
        assert_eq!(l.last_idx(), 0);
        assert_eq!(l.app.get_state(), 0);
    }

    #[test]
    fn last_term_and_index_of_non_empty() {
        let app = setup();
        let mut l: Log<u32, u32> = Log::new(Box::new(app));
        l.entries.push(LogEntry { term: 0, data: 1 });
        l.entries.push(LogEntry { term: 0, data: 2 });
        assert_eq!(l.last_term(), 0);
        assert_eq!(l.last_idx(), 1);

        l.entries.push(LogEntry { term: 1, data: 3 });
        assert_eq!(l.last_term(), 1);
        assert_eq!(l.last_idx(), 2);
    }
}
