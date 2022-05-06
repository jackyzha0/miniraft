use std::collections::{BTreeMap, BTreeSet};

use crate::log::{Log, LogIndex};

/// Type alias for Raft leadership term
pub type Term = u64;

/// Type alias for the ID of a single Raft server
pub type ServerId = usize;

type Ticks = u32;

/// Configuration options for a Raft server
#[derive(Debug)]
pub struct RaftConfig {
    /// How long a server should wait for a message from
    /// current leader before giving up and starting an election
    pub election_timeout: Ticks,

    /// How much random jitter to add to [`election_timeout`](Self::election_timeout)
    pub election_timeout_jitter: Ticks,

    /// How often a leader should send empty 'heartbeat' AppendEntry RPC
    /// calls to maintain power. Generally one magnitude smaller than [`election_timeout`](Self::election_timeout)
    pub heartbeat_interval: Ticks,
}

/// A Raft server that replicates Logs of type `T`
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

    /// State of the node that depends on its leadership status
    /// (one of [`FollowerState`], [`CandidateState`], or [`LeaderState`])
    leadership_state: RaftLeadershipState,
}

/// Possible states a Raft Node can be in
#[derive(Debug)]
pub enum RaftLeadershipState {
    /// Issues no requests but responds to requests from leaders and candidates.
    /// All Raft Nodes start in Follower state
    Follower(FollowerState),

    /// Used to elect a new leader.
    Candidate(CandidateState),

    /// Handles all client requests.
    Leader(LeaderState),
}

impl RaftLeadershipState {
    fn tick(&mut self) {}
}

#[derive(Debug)]
pub struct FollowerState {
    /// Current leader node is following
    leader: Option<ServerId>,
    /// Ticks left to start an election if not reset by activity/heartbeat
    election_time: Ticks,
}

#[derive(Debug)]
pub struct CandidateState {
    /// Set of all nodes this node has received votes for
    votes_received: BTreeSet<ServerId>,
    /// Ticks left to start an election if quorum is not reached
    election_time: Ticks,
}

#[derive(Debug)]
pub struct LeaderState {
    /// Track state about followers to figure out what to send them next
    followers: BTreeMap<ServerId, NodeReplicationState>,
    /// Ticks left till when to send the next heartbeat
    heartbeat_timeout: Ticks,
}

/// State of a single Node as tracked by a leader
#[derive(Debug)]
pub struct NodeReplicationState {
    /// Index of next log entry to send to that server.
    /// Initialized to leader's last log index + 1
    pub next_idx: LogIndex,

    /// Index of highest log entry known to be replicated on server.
    /// Initialized to 0, increases monotonically
    pub match_idx: LogIndex,
}
