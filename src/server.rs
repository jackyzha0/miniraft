use crate::{
    log::{Log, LogIndex},
    rpc::{Target, VoteRequest, RPC},
    transport::TransportMedium,
};
use anyhow::Result;
use rand::Rng;
use rand_chacha::ChaCha8Rng;
use rand_core::SeedableRng;
use std::collections::{BTreeMap, BTreeSet};

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

/// A Raft server that replicates Logs of type `T`
pub struct RaftServer<'s, T> {
    // Static State
    /// ID of this node
    id: ServerId,
    /// All other servers in this Raft cluster
    peers: BTreeSet<ServerId>,
    /// Config of this node
    config: RaftConfig,

    // Persistent State
    /// Current term of this node
    current_term: Term,
    /// Candidate node that we voted for this election
    voted_for: Option<ServerId>,
    /// List of log entries for this node.
    /// This is the data that is being replicated
    log: Log<T>,

    /// State of the node that depends on its leadership status
    /// (one of [`FollowerState`], [`CandidateState`], or [`LeaderState`])
    leadership_state: RaftLeadershipState,

    /// Internal seeded random number generator
    rng: ChaCha8Rng,

    /// Underlying message medium
    transport_layer: &'s mut dyn TransportMedium<T>,
}

impl<'s, T> RaftServer<'s, T> {
    pub fn new(
        id: ServerId,
        peers: BTreeSet<ServerId>,
        config: RaftConfig,
        seed: Option<u64>,
        transport_layer: &'s mut dyn TransportMedium<T>,
    ) -> Self {
        // Create RNG generator from seed if it exists, otherwise seed from system entropy
        let mut rng = match seed {
            Some(n) => ChaCha8Rng::seed_from_u64(n),
            None => ChaCha8Rng::from_entropy(),
        };

        // Set random election time
        let random_election_time = rng_jitter(
            &mut rng,
            config.election_timeout,
            config.election_timeout_jitter,
        );

        // Initialize state, set leadership state to follower
        RaftServer {
            id,
            peers,
            config,
            current_term: 0,
            voted_for: None,
            log: Log::new(),
            rng,
            transport_layer,
            leadership_state: RaftLeadershipState::Follower(FollowerState {
                leader: None,
                election_time: random_election_time,
            }),
        }
    }

    /// Helper function to generate a random election time given current configuration
    fn random_election_time(&mut self) -> Ticks {
        rng_jitter(
            &mut self.rng,
            self.config.election_timeout,
            self.config.election_timeout_jitter,
        )
    }

    /// Tick state and perform necessary state transitions/RPC calls
    pub fn tick(&mut self) -> &mut Self {
        use RaftLeadershipState::*;
        match &mut self.leadership_state {
            Follower(state) => {
                state.election_time = state.election_time.saturating_sub(1);

                // suspect leader has failed, election timeout reached
                // attempt to become candidate
                if state.election_time == 0 {
                    // vote for self
                    self.voted_for = Some(self.id);
                    let mut vote_list = BTreeSet::new();
                    vote_list.insert(self.id);

                    // set state to candidate
                    self.leadership_state = Candidate(CandidateState {
                        election_time: self.random_election_time(),
                        votes_received: vote_list,
                    });

                    // broadcast message to all nodes asking for a vote
                    let rpc = RPC::VoteRequest(VoteRequest {
                        candidate_term: self.current_term,
                        candidate_id: self.id,
                        candidate_last_log_idx: self.log.entries.len(),
                        candidate_last_log_term: self.log.last_term(),
                    });
                    let msg = (Target::Broadcast, rpc);
                    self.transport_layer
                        .send(&msg)
                        .expect("invalid recipient server")
                }
            }
            Candidate(state) => {
                // TODO: stub
            }
            Leader(state) => {
                // TODO: stub
            }
        }
        self
    }

    pub fn receive_rpc(&mut self, rpc: &RPC<T>) {
        match rpc {
            RPC::VoteRequest(req) => {
                if req.candidate_term > self.current_term {
                    // we are out of date!
                    // set candidate back to follower state and bump current term
                    self.current_term = req.candidate_term;
                    self.voted_for = None;
                    self.leadership_state = RaftLeadershipState::Follower(FollowerState {
                        leader: None, // as we are in an election
                        election_time: self.random_election_time(),
                    })
                }

                // check if candidate's log is up to date with ours
                // if they are outdated, don't vote for them (we don't want an outdated leader)
            }
            RPC::AppendRequest(req) => {
                // TODO: stub
            }
        }
    }
}

/// Returns a random u32 uniformly from (expected)
fn rng_jitter(rng: &mut ChaCha8Rng, expected: u32, jitter: u32) -> u32 {
    let low = expected - jitter;
    let hi = expected + jitter;
    rng.gen_range(low..=hi)
}
