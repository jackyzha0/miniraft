/// Crate for making pretty looking outputs for testing and debugging
use crate::{
    log::{Log, LogEntry, LogIndex},
    rpc::{AppendRequest, AppendResponse, SendableMessage, Target, VoteRequest, VoteResponse, RPC},
    server::{NodeReplicationState, RaftServer, ServerId, Term},
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

pub struct Logger {}
impl Logger {
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

    pub fn server_init<T: Debug + Clone, S>(raft_ref: &RaftServer<T, S>) {
        log(
            &raft_ref.id,
            "initializing server".to_owned(),
            Level::Overview,
        );
        Self::state_update(raft_ref);
    }

    pub fn state_update<T: Debug + Clone, S>(raft_ref: &RaftServer<T, S>) {
        let state_str = if raft_ref.is_leader() {
            " Leader ".on_blue()
        } else if raft_ref.is_candidate() {
            " Candidate ".on_yellow()
        } else {
            " Follower ".on_truecolor(140, 140, 140)
        }
        .bold()
        .black()
        .to_string();
        log(
            &raft_ref.id,
            format!("is now {}", state_str),
            Level::Overview,
        );
    }

    pub fn won_election<T: Debug + Clone, S>(
        raft_ref: &RaftServer<T, S>,
        num_votes: usize,
        follower_ids: &Vec<ServerId>,
    ) {
        Self::state_update(raft_ref);
        log(
            &raft_ref.id,
            format!(
                "won election with votes={} out of quorum={}, followers: {}",
                num_votes,
                raft_ref.quorum_size(),
                follower_ids
                    .iter()
                    .map(|id| colour_server(id))
                    .collect::<Vec<String>>()
                    .join(", ")
            ),
            Level::Requests,
        );
    }

    pub fn send_heartbeat<T: Debug + Clone, S>(raft_ref: &RaftServer<T, S>) {
        log(
            &raft_ref.id,
            "sending heartbeat to all followers".to_owned(),
            Level::Trace,
        );
    }

    pub fn election_timer_expired<T: Debug + Clone, S>(raft_ref: &RaftServer<T, S>) {
        log(
            &raft_ref.id,
            format!(
                "election timer expired, bumped to {} and started election",
                colour_term(raft_ref.current_term)
            ),
            Level::Overview,
        );
    }

    pub fn outgoing_rpcs<T: Debug + Clone, S>(
        raft_ref: &RaftServer<T, S>,
        msgs: Vec<SendableMessage<T>>,
    ) -> Vec<SendableMessage<T>> {
        msgs.iter().for_each(|msg| {
            log(
                &raft_ref.id,
                match &msg {
                    (Target::Single(target), rpc) => format!("{rpc} -> {}", colour_server(target)),
                    (Target::Broadcast, rpc) => {
                        format!("{rpc} -> {}", " All servers ".bold().black().on_white())
                    }
                },
                Level::Overview,
            )
        });
        msgs
    }

    pub fn check_matching_term<T>(id: &ServerId, req: &AppendRequest<T>, current_term: Term) {
        log(
            id,
            format!(
                "checking pre-req, does our term match the leader's term: {}",
                colour_bool(req.leader_term == current_term),
            ),
            Level::Trace,
        );
    }

    pub fn bumping_term<T: Debug + Clone, S>(raft_ref: &RaftServer<T, S>, new_term: Term) {
        log(
            &raft_ref.id,
            format!(
                "bumping our term {} to match candidate/leader term {}",
                colour_term(raft_ref.current_term),
                colour_term(new_term)
            ),
            Level::Trace,
        );
    }

    pub fn receive_rpc<T: Debug + Clone, S>(raft_ref: &RaftServer<T, S>, rpc: &RPC<T>) {
        log(&raft_ref.id, format!("<- {rpc}"), Level::Overview);
    }

    pub fn client_request<T: Debug + Clone, S>(raft_ref: &RaftServer<T, S>) {
        log(
            &raft_ref.id,
            "received client_request to add an entry".to_owned(),
            Level::Overview,
        );
    }

    pub fn replicate_entries<T: Debug + Clone, S>(
        raft_ref: &RaftServer<T, S>,
        entries: &Vec<LogEntry<T>>,
        target: &ServerId,
        prefix_len: LogIndex,
    ) {
        if entries.len() == 0 {
            log(
                &raft_ref.id,
                format!("preparing heartbeat signal to {}", colour_server(target)),
                Level::Trace,
            );
        } else {
            log(
                &raft_ref.id,
                format!(
                    "preparing RPC call to {}... replicating a portion of our log: {}",
                    colour_server(target),
                    debug_log(
                        &raft_ref.log.entries,
                        vec![(
                            AnnotationType::Span(prefix_len, raft_ref.log.entries.len()),
                            "these entries"
                        )],
                        0
                    ),
                ),
                Level::Trace,
            );
        }
    }

    pub fn rpc_vote_request<T: Debug + Clone, S>(raft_ref: &RaftServer<T, S>, req: &VoteRequest) {
        log(
            &raft_ref.id,
            format!(
                "[rpc_vote_request] from {}",
                colour_server(&req.candidate_id)
            ),
            Level::Requests,
        );
    }

    pub fn rpc_vote_result<T: Debug + Clone, S>(
        raft_ref: &RaftServer<T, S>,
        log_ok: bool,
        up_to_date: bool,
        havent_voted: bool,
    ) {
        log(
            &raft_ref.id,
            format!(
                "vote: {} because\n1) their log has a more recent term or is longer: {}\n2) their term is up to date: {}\n3) we haven't voted this election cycle or we already voted for them: {}",
                colour_bool(log_ok && up_to_date && havent_voted),
                colour_bool(log_ok),
                colour_bool(up_to_date),
                colour_bool(havent_voted)
            ),
            Level::Trace,
        );
    }

    pub fn rpc_vote_resp<T: Debug + Clone, S>(raft_ref: &RaftServer<T, S>, res: &VoteResponse) {
        log(
            &raft_ref.id,
            format!(
                "[rpc_vote_response] from {} voting {}",
                colour_server(&res.votee_id),
                colour_bool(res.vote_granted)
            ),
            Level::Requests,
        );
    }

    pub fn vote_count(id: &ServerId, res: &VoteResponse, up_to_date: bool) {
        log(
            id,
            format!(
                "counting vote: {} because\n1) follower is up to date: {}\n2) they granted the vote: {}",
                colour_bool(up_to_date && res.vote_granted),
                colour_bool(up_to_date),
                colour_bool(res.vote_granted),
            ),
            Level::Trace,
        );
    }

    pub fn total_vote_count(id: &ServerId, total: usize, quorum: usize) {
        log(
            id,
            format!("total vote count: {} out of quorum of {}", total, quorum,),
            Level::Trace,
        );
    }

    pub fn added_follower<T: Debug + Clone, S>(raft_ref: &RaftServer<T, S>, votee: &ServerId) {
        log(
            &raft_ref.id,
            format!("added {} to list of followers", colour_server(votee)),
            Level::Trace,
        )
    }

    pub fn rpc_append_request<T: Debug + Clone, S>(
        raft_ref: &RaftServer<T, S>,
        req: &AppendRequest<T>,
    ) {
        log(
            &raft_ref.id,
            format!(
                "[rpc_append_request] from {}",
                colour_server(&req.leader_id),
            ),
            Level::Requests,
        );
    }

    pub fn append_conflict_check<T: Debug + Clone, S>(
        raft_ref: &RaftServer<T, S>,
        req: &AppendRequest<T>,
    ) {
        log(
            &raft_ref.id,
            format!(
                "comparing our term {} to supposed leader {}",
                colour_term(raft_ref.current_term),
                colour_term(req.leader_term)
            ),
            Level::Trace,
        );

        if req.leader_term == raft_ref.current_term {
            log(
                &raft_ref.id,
                format!("term matches, reset to follower, update term and retry"),
                Level::Trace,
            );
        } else {
            log(&raft_ref.id, format!("outdated, ignoring"), Level::Trace);
        }
    }

    pub fn append_entries<T: Debug + Clone, S>(
        raft_ref: &RaftServer<T, S>,
        prefix_ok: bool,
        last_log_entry_matches_terms: bool,
        prefix_len: usize,
    ) {
        log(&raft_ref.id, format!(
            "append entries: {} because\n1) the index they want to insert entries at ({}) <= our log length ({}): {}\n2) last log entry before new entries matches terms: {}",
            colour_bool(prefix_ok && last_log_entry_matches_terms),
            prefix_len,
            raft_ref.log.entries.len(),
            colour_bool(prefix_ok),
            colour_bool(last_log_entry_matches_terms)
        ), Level::Trace);
    }

    pub fn append_response<T: Debug + Clone, S>(raft_ref: &RaftServer<T, S>, res: &AppendResponse) {
        log(
            &raft_ref.id,
            format!(
                "[rpc_append_response] from {}",
                colour_server(&res.follower_id),
            ),
            Level::Requests,
        );
    }

    pub fn process_append_response(
        id: &ServerId,
        res: &AppendResponse,
        follower_state: &NodeReplicationState,
    ) {
        let valid = res.ok && res.ack_idx >= follower_state.acked_up_to;
        log(
            id,
            format!(
                "valid response: {} because\n1) response indicated success: {}\n2) the index they acked up to (new={}) actually moved forward (old={}): {}",
                colour_bool(valid),
                colour_bool(res.ok),
                res.ack_idx,
                follower_state.acked_up_to,
                res.ack_idx >= follower_state.acked_up_to,
            ),
            Level::Trace,
        );

        let msg = if valid {
            format!(
                "success! bumping sent_up_to from {} -> {} and acked_up_to from {} -> {}",
                follower_state.sent_up_to, res.ack_idx, follower_state.acked_up_to, res.ack_idx
            )
        } else {
            format!(
                "error, decrement sent_up_to from {} -> {} and try again",
                follower_state.sent_up_to,
                follower_state.sent_up_to - 1,
            )
        };

        log(id, msg, Level::Trace)
    }

    pub fn commit_entry(id: &ServerId, commit_len: LogIndex, acks: usize, quorum_size: usize) {
        log(id, format!(
            "commit entry at index ({}): {} because\n1) total of {} acks for that index >= quorum size ({})",
            commit_len,
            colour_bool(acks >= quorum_size),
            acks,
            quorum_size,
        ), Level::Trace);
    }
}
