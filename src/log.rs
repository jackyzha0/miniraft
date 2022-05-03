use std::collections::VecDeque;

use crate::server::Term;

pub type LogIndex = u64;

#[derive(Clone, Debug)]
pub struct LogEntry<T> {
    pub term: Term,
    pub data: T,
}

/// A collection of LogEntries
#[derive(Debug)]
pub struct Log<T> {
    entries: VecDeque<LogEntry<T>>,
}

impl<T> Log<T> {}
