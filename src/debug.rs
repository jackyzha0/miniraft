use crate::{
    log::{LogEntry, LogIndex},
    server::{ServerId, Term},
};
use colored::Colorize;
use core::fmt;
use env_logger::TimestampPrecision;
use log::{debug, info, trace};
use random_color::{Luminosity, RandomColor};

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

pub fn info(id: &ServerId, msg: String) {
    log(id, msg, Level::Overview);
}

pub fn debug(id: &ServerId, msg: String) {
    log(id, msg, Level::Requests);
}

pub fn trace(id: &ServerId, msg: String) {
    log(id, msg, Level::Trace);
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
