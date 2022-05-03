use std::{
    collections::{BTreeMap, BTreeSet},
    time::{Duration, Instant},
};

use crate::log::{Log, LogIndex};

pub type Term = u64;
pub type ServerId = u64;

#[derive(Debug)]
pub struct RaftConfig {
    pub election_timeout: Duration,
    pub heartbeat_interval: Duration,
    pub replication_chunk_size: usize,
}

#[derive(Debug)]
pub struct RaftNode<T> {
    /// Static State
    id: ServerId,
    peers: BTreeSet<ServerId>,
    config: RaftConfig,

    /// Persistent State
    current_term: Term,
    voted_for: Option<ServerId>,
    log: Log<T>,

    /// Volatile State
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
