use crate::log::{LogEntry, LogIndex};
use crate::server::{RaftServer, ServerId, Term};

#[derive(Debug)]
pub enum RPC<T> {
    VoteRequest(VoteRequest),
    VoteResponse(VoteResponse),
    AppendRequest(AppendRequest<T>),
    AppendResponse(AppendResponse),
}

/// Request by a candidate to become a Raft leader
#[derive(Debug)]
pub struct VoteRequest {
    /// Current term of candidate
    pub candidate_term: Term,
    /// ID of candidate requesting a vote
    pub candidate_id: ServerId,
    /// Index of candidate's last log entry
    pub candidate_last_log_idx: LogIndex,
    /// Term of candidate's last log entry
    pub candidate_last_log_term: Term,
}

/// Response to a [`VoteRequest`]
#[derive(Debug)]
pub struct VoteResponse {
    /// [`current_term`](RaftServer::current_term) of server for candidate to update itself
    pub term: Term,
    /// Whether the [`VoteRequest`] was granted or not
    pub vote_granted: bool,
}

/// Request from leader to append entries to follower's log
#[derive(Debug)]
pub struct AppendRequest<T> {
    /// Term of leader requesting log append
    pub leader_term: Term,
    /// ID of leader (used so follower can redirect clients)
    pub leader_id: ServerId,
    /// Log index immediately preceding index of first element in [`entries`](Self::entries)
    pub prev_entries_idx: LogIndex,
    pub prev_entries_term: Term,
    pub leader_commit: LogIndex,
    /// A list of consecutive log entries to append to follower
    pub entries: Vec<LogEntry<T>>,
}

/// Response to an [`AppendRequest`]
#[derive(Debug)]
pub struct AppendResponse {
    pub ok: bool,
    pub match_idx: LogIndex,
    pub last_log_idx: LogIndex,
}
