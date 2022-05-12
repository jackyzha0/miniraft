use crate::{
    log::{LogEntry, LogIndex},
    server::ServerId,
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

pub fn log(id: &ServerId, msg: String, level: Level) {
    let [r, g, b] = RandomColor::new()
        .luminosity(Luminosity::Light)
        .seed(*id as u32)
        .to_rgb_array();
    let id_prefix = format!(" Server {} ", id).black().on_truecolor(r, g, b);
    let fmt_msg = format!("{} {}{}", id_prefix, level, msg.dimmed());
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
    Index,
    Length,
}
pub type Annotation = (LogIndex, AnnotationType, &'static str);
pub fn debug_log<T: fmt::Debug>(
    entries: &Vec<LogEntry<T>>,
    annotations: Vec<Annotation>,
    log_offset: LogIndex,
) -> String {
    let strs: Vec<String> = entries
        .iter()
        .map(|LogEntry { term, data }| format!("({}) {:?}", term, data))
        .collect();
    let first_line = format!("{}{}\n", " ".repeat(9 * log_offset), strs.join(" -> "));

    let annotation_lines = annotations
        .iter()
        .map(|(i, annotation_type, msg)| match annotation_type {
            AnnotationType::Index => {
                let applied_padding = " ".repeat(9 * (i + log_offset));
                format!("{applied_padding}    ^ {msg}")
            }
            AnnotationType::Length => {
                if *i == 0 {
                    format!("|  {msg}")
                } else {
                    let applied_padding = "~~~~~~~~~"
                        .repeat(*i + log_offset)
                        .get(0..(9 * (*i) + log_offset) - 4)
                        .unwrap()
                        .to_string();
                    format!("{applied_padding}|  {msg}")
                }
            }
        })
        .collect::<Vec<String>>()
        .join("\n");

    format!("\n{}{}", first_line, annotation_lines)
}
