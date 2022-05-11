use miniraft::debug::*;
use miniraft::log::{App, Log, LogEntry};

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

#[test]
fn apply_to_state_basic() {
    let app = setup();
    let mut l: Log<u32, u32> = Log::new(Box::new(app));
    l.entries.push(LogEntry { term: 0, data: 5 });
    l.deliver_msg();
    assert_eq!(l.last_applied, 1);
    assert_eq!(l.app.get_state(), 5);

    l.entries.push(LogEntry { term: 1, data: 12 });
    l.entries.push(LogEntry { term: 3, data: 2 });
    assert_eq!(l.last_applied, 1);
    assert_eq!(l.app.get_state(), 5);
    assert_eq!(l.last_term(), 3);
    assert_eq!(l.last_idx(), 2);

    l.deliver_msg();
    l.deliver_msg();
    assert_eq!(l.last_applied, l.last_idx());
    assert_eq!(l.app.get_state(), 5 + 12 + 2);
}
