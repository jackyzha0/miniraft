use std::{
    collections::{BTreeMap, BTreeSet},
    time::{Duration, Instant},
};

use crate::log::{Log, LogIndex};

/// Type alias for Raft leadership term
pub type Term = u64;

/// Type alias for the ID of a single Raft server
pub type ServerId = u64;

/// Configuration options for a Raft server
#[derive(Debug)]
pub struct RaftConfig {
    /// How long a server should wait for a message from
    /// current leader before giving up and starting an election
    pub election_timeout: Duration,

    /// How often a leader should send empty 'heartbeat' AppendEntry RPC
    /// calls to maintain power
    pub heartbeat_interval: Duration,
}

#[derive(Debug)]
pub struct RaftServer<T> {
    // Static State
    /// ID of this node
    pub id: ServerId,
    /// All other servers in this Raft cluster
    pub peers: BTreeSet<ServerId>,
    /// Config of this node
    pub config: RaftConfig,

    // Persistent State
    /// Current term of this node
    pub current_term: Term,
    /// Candidate node that we voted for this election
    voted_for: Option<ServerId>,
    /// List of log entries for this node.
    /// This is the data that is being replicated
    log: Log<T>,

    // Volatile State
    /// State of the node that depends on its leadership status
    /// (one of [`FollowerState`], [`CandidateState`], or [`LeaderState`])
    leadership_state: RaftLeadershipState,
}

#[derive(Debug)]
pub enum RaftLeadershipState {
    Follower(FollowerState),
    Candidate(CandidateState),
    Leader(LeaderState),
}

#[derive(Debug)]
pub struct FollowerState {
    leader: Option<ServerId>,
    random_election_timeout: Duration,
    election_time: Instant,
}

#[derive(Debug)]
pub struct CandidateState {
    votes_received: BTreeSet<ServerId>,
    election_time: Instant,
}

#[derive(Debug)]
pub struct LeaderState {
    followers: BTreeMap<ServerId, ReplicationState>,
    heartbeat_timeout: Instant,
}

#[derive(Debug)]
pub struct ReplicationState {
    pub next_idx: LogIndex,
    pub match_idx: LogIndex,
    pub next_heartbeat: Instant,
}
