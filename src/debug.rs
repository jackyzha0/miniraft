/// Crate for making pretty looking outputs for testing and debugging
use crate::{
    log::{Log, LogEntry, LogIndex},
    server::{ServerId, Term},
};
use colored::Colorize;
use core::fmt;
use env_logger::TimestampPrecision;
use log::{debug, info, trace};
use random_color::{Luminosity, RandomColor};
use std::fmt::Debug;

pub enum Level {
    // State transitions + RPCs
    Overview,
    // + function calls
    Requests,
    // inner function workings
    Trace,
}

impl fmt::Display for Level {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{}",
            match &self {
                Level::Overview => "",
                Level::Requests => "  ",
                Level::Trace => "    ",
            }
        )
    }
}

pub fn init_logger() {
    println!("");
    let _ = env_logger::builder()
        .is_test(true)
        .format_module_path(false)
        .format_target(false)
        .format_level(false)
        .format_timestamp(Some(TimestampPrecision::Micros))
        .format_indent(Some(45))
        .try_init();
}

pub fn colour_server(id: &ServerId) -> String {
    let [r, g, b] = RandomColor::new()
        .luminosity(Luminosity::Light)
        .seed(*id as u32 + 4)
        .to_rgb_array();
    format!(" Server {} ", id)
        .black()
        .on_truecolor(r, g, b)
        .to_string()
}

pub fn colour_term(term: Term) -> String {
    format!(" Term {} ", term)
        .bold()
        .black()
        .on_white()
        .to_string()
}

pub fn colour_bool(b: bool) -> String {
    match b {
        true => "ok".green(),
        false => "no".red(),
    }
    .bold()
    .to_string()
}

pub fn log(id: &ServerId, msg: String, level: Level) {
    let fmt_msg = format!("{} {}{}", colour_server(id), level, msg.dimmed());
    match level {
        Level::Overview => info!("{}", fmt_msg),
        Level::Requests => debug!("{}", fmt_msg),
        Level::Trace => trace!("{}", fmt_msg),
    }
}

/// Internal debug message to dump contents of entries and state
pub enum AnnotationType {
    Index(usize),
    Length(usize),
    Span(usize, usize),
}
pub type Annotation = (AnnotationType, &'static str);
pub fn debug_log<T: fmt::Debug>(
    entries: &Vec<LogEntry<T>>,
    annotations: Vec<Annotation>,
    log_offset: LogIndex,
) -> String {
    let strs: Vec<String> = entries
        .iter()
        .map(|LogEntry { term, data }| format!("({}) {:?}", term, data))
        .collect();
    let sep = if annotations.len() > 0 { "\n" } else { "" };
    let first_line = format!("{}{}{}", " ".repeat(9 * log_offset), strs.join(" -> "), sep);

    let annotation_lines = annotations
        .iter()
        .map(|(annotation_type, msg)| match annotation_type {
            AnnotationType::Index(i) => {
                let applied_padding = " ".repeat(9 * (i + log_offset));
                format!("{applied_padding}    ^ {msg}")
            }
            AnnotationType::Length(length) => {
                if *length == 0 {
                    format!("|  {msg}")
                } else {
                    let applied_padding = "~~~~~~~~~"
                        .repeat(length + log_offset)
                        .get(0..(9 * (length) + log_offset) - 4)
                        .unwrap()
                        .to_string();
                    format!("{applied_padding}|  {msg}")
                }
            }
            AnnotationType::Span(start, end) => {
                if end - start == 0 {
                    format!("|  {msg}")
                } else {
                    let pre_padding = " ".repeat(9 * (log_offset + start));
                    let applied_padding = "~~~~~~~~~"
                        .repeat(end - start)
                        .get(1..(9 * (end - start - 4)))
                        .unwrap()
                        .to_string();
                    format!("{pre_padding}|{applied_padding}|  {msg}")
                }
            }
        })
        .collect::<Vec<String>>()
        .join("\n");

    format!("\n{}{}", first_line, annotation_lines)
}

pub struct Tracer {}
impl Tracer {
    pub fn append_entries_recv<T: Debug, S>(
        log_ref: &Log<T, S>,
        prefix_idx: LogIndex,
        leader_commit_len: LogIndex,
        their_entries: &Vec<LogEntry<T>>,
    ) {
        let msg = if their_entries.len() > 0 {
            format!(
                "[append_entries] received with prefix_idx={}, leader_commit_len={}\ncurrent state: {}\nentries to append:{}",
                prefix_idx,
                leader_commit_len,
                debug_log(&log_ref.entries, Vec::new(), 0),
                debug_log(&their_entries, Vec::new(), prefix_idx)
            )
        } else {
            format!(
                "[append_entries] received heartbeat prefix_idx={}",
                prefix_idx
            )
        };

        log(&log_ref.parent_id, msg, Level::Requests)
    }

    pub fn log_potential_conflict<T: Debug, S>(
        log_ref: &Log<T, S>,
        their_entries: &Vec<LogEntry<T>>,
        prefix_idx: LogIndex,
        rollback_to: LogIndex,
    ) {
        log(
            &log_ref.parent_id,
            format!(
                "potential log conflict! compare our terms\nour log: {}\nentries to append (attempting to insert at idx={}): {}",
                debug_log(
                    &log_ref.entries,
                    vec![(AnnotationType::Index(rollback_to), "term of this entry")],
                    0,
                ),
                prefix_idx,
                debug_log(
                    &their_entries,
                    vec![(
                        AnnotationType::Index(rollback_to - prefix_idx),
                        "term of this entry leader is trying to add"
                    )],
                    prefix_idx
                )
            ),
            Level::Trace,
        );
    }

    pub fn log_term_conflict<T: Debug, S>(log_ref: &Log<T, S>) {
        log(
            &log_ref.parent_id,
            format!(
                "term conflict detected! truncating our log to length={} to match leader",
                log_ref.entries.len(),
            ),
            Level::Trace,
        );
    }

    pub fn log_append<T: Debug, S>(log_ref: &Log<T, S>, start: LogIndex) {
        log(
            &log_ref.parent_id,
            format!(
                "appended all request entries starting from idx={}, log now looks like: {}",
                start,
                debug_log(&log_ref.entries, Vec::new(), 0)
            ),
            Level::Trace,
        );
    }

    pub fn log_apply<T: Debug, S>(log_ref: &Log<T, S>, leader_commit_len: LogIndex) {
        log(
            &log_ref.parent_id,
            format!(
                "applied more messages to state machine: {}",
                debug_log(
                    &log_ref.entries,
                    vec![
                        (
                            AnnotationType::Length(log_ref.committed_len),
                            "used to be commited up to here"
                        ),
                        (
                            AnnotationType::Length(leader_commit_len),
                            "now commited up to here"
                        ),
                    ],
                    0
                ),
            ),
            Level::Trace,
        )
    }

    pub fn log_deliver_recv<T: Debug, S>(log_ref: &Log<T, S>) {
        log(
            &log_ref.parent_id,
            format!(
                "[deliver_msg] at applied_idx={} out of entries.len()={}",
                log_ref.applied_len,
                log_ref.entries.len()
            ),
            Level::Requests,
        );
    }

    pub fn log_deliver_apply<T: Debug, S>(log_ref: &Log<T, S>) {
        log(
            &log_ref.parent_id,
            debug_log(
                &log_ref.entries,
                vec![(
                    AnnotationType::Length(log_ref.applied_len),
                    "applied up to here",
                )],
                0,
            ),
            Level::Trace,
        );
    }
}
