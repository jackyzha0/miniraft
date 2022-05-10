use crate::log::{LogEntry, LogIndex};
use crate::server::{RaftServer, ServerId, Term};

/// A message can be either targeted at a single server or to everyone
pub type SendableMessage<T> = (Target, RPC<T>);

/// Whether to send a message to everyone or just a single node
pub enum Target {
    Single(ServerId),
    Broadcast,
}

/// A Raft RPC request
pub enum RPC<T> {
    VoteRequest(VoteRequest),
    AppendRequest(AppendRequest<T>),
    VoteResponse(VoteResponse),
    AppendResponse(AppendResponse),
}

/// Request by a candidate to become a Raft leader
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
pub struct VoteResponse {
    /// [`current_term`](RaftServer::current_term) of server for candidate to update itself
    pub term: Term,
    /// Whether the [`VoteRequest`] was granted or not
    pub vote_granted: bool,
    /// Who sent the vote
    pub votee_id: ServerId,
}

/// Request from leader to append entries to follower's log
#[derive(Clone)]
pub struct AppendRequest<T> {
    /// Term of leader requesting log append
    pub leader_term: Term,
    /// ID of leader (used so follower can redirect clients)
    pub leader_id: ServerId,
    /// Log index immediately preceding index of next element in [`entries`](Self::entries)
    pub leader_last_log_idx: LogIndex,
    /// Term of [`leader_last_log_idx`](Self::prev_entries_idx)
    pub leader_last_log_term: Term,
    /// Leader's [`commit_idx`](Log::commit_idx)
    pub leader_commit: LogIndex,
    /// A list of consecutive log entries to append to follower
    pub entries: Vec<LogEntry<T>>,
}

/// Response to an [`AppendRequest`]
pub struct AppendResponse {
    pub ok: bool,
}
