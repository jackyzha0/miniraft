use crate::server::ServerId;
use colored::Colorize;
use core::fmt;
use env_logger::TimestampPrecision;
use log::{debug, info, trace};
use random_color::{Luminosity, RandomColor};

pub enum Level {
    // State transitions
    Overview,
    // + RPC send/receive
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
