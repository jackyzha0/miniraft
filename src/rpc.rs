use std::fmt::{Display, Formatter, Result};

use crate::log::*;
use crate::server::*;

/// A message can be either targeted at a single server or to everyone
pub type SendableMessage<T> = (Target, RPC<T>);

/// Whether to send a message to everyone or just a single node
#[derive(Eq, PartialEq, PartialOrd, Ord, Clone, Copy)]
pub enum Target {
    /// A single server
    Single(ServerId),
    /// To everyone
    Broadcast,
}

/// A Raft RPC request
pub enum RPC<T> {
    /// Candidate requesting to become leader
    VoteRequest(VoteRequest),
    /// Response to [`VoteRequest`]
    VoteResponse(VoteResponse),
    /// Leader heartbeat/appending entries to followers
    AppendRequest(AppendRequest<T>),
    /// Response to [`AppendRequest`]
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
    /// Term of [`leader_last_log_idx`](Self::leader_last_log_idx)
    pub leader_last_log_term: Term,
    /// Leader's [`committed_len`](Log::committed_len)
    pub leader_commit: LogIndex,
    /// A list of consecutive log entries to append to follower
    pub entries: Vec<LogEntry<T>>,
}

/// Response to an [`AppendRequest`]
pub struct AppendResponse {
    /// Whether the follower added it to their log or not
    pub ok: bool,
    /// [`current_term`](RaftServer::current_term) of server for candidate to update itself
    pub term: Term,
    /// Index of the last log entry we appended to the log
    pub ack_idx: LogIndex,
    /// Follower ID
    pub follower_id: ServerId,
}

/// Display trait implementations
impl<T> Display for RPC<T> {
    fn fmt(&self, f: &mut Formatter) -> Result {
        write!(
            f,
            "{}",
            match self {
                RPC::VoteRequest(_) => "VoteRequest",
                RPC::AppendRequest(_) => "AppendRequest",
                RPC::VoteResponse(_) => "VoteResponse",
                RPC::AppendResponse(_) => "AppendResponse",
            }
        )
    }
}
