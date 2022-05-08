use crate::{
    log::{Log, LogIndex},
    rpc::{AppendRequest, Target, VoteRequest, VoteResponse, RPC},
    transport::TransportMedium,
};
use rand::Rng;
use rand_chacha::ChaCha8Rng;
use rand_core::SeedableRng;
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Div,
};

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

impl PartialEq for RaftLeadershipState {
    fn eq(&self, other: &Self) -> bool {
        use RaftLeadershipState::*;
        match (self, other) {
            (Follower(_), Follower(_)) => true,
            (Candidate(_), Candidate(_)) => true,
            (Leader(_), Leader(_)) => true,
            (_, _) => false,
        }
    }
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
    pub sent_up_to: LogIndex,

    /// Index of highest log entry known to be replicated on server.
    /// Initialized to 0, increases monotonically
    pub acked_up_to: LogIndex,
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

impl<'s, T> RaftServer<'s, T>
where
    T: Clone,
{
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
                        candidate_last_log_idx: self.log.last_idx(),
                        candidate_last_log_term: self.log.last_term(),
                    });
                    let msg = (Target::Broadcast, rpc);
                    self.transport_layer.send(&msg).unwrap()
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

    fn reset_to_follower(&mut self, new_term: Term) {
        self.current_term = new_term;
        self.voted_for = None;
        self.leadership_state = RaftLeadershipState::Follower(FollowerState {
            leader: None, // as we are in an election
            election_time: self.random_election_time(),
        })
    }

    /// Calculate quorum of current set of peers.
    /// quorum = ceil((peers.length + 1)/2)
    fn quorum_size(&self) -> usize {
        // add an extra because self.peers doesn't include self
        self.peers.len().saturating_add(2).div(2)
    }

    pub fn receive_rpc(&mut self, rpc: &RPC<T>) {
        use RaftLeadershipState::*;
        match rpc {
            RPC::VoteRequest(req) => {
                if req.candidate_term > self.current_term {
                    // if we are behind the other candidate, just reset to follower
                    self.reset_to_follower(req.candidate_term);
                }

                // check if candidate's log is up to date with ours
                // if they are outdated, don't vote for them (we don't want an outdated leader)
                let candidate_has_more_recent_log =
                    req.candidate_last_log_term > self.log.last_term();
                let candidate_has_longer_log = req.candidate_last_log_term == self.log.last_term()
                    && req.candidate_last_log_idx >= self.log.last_idx();
                let log_ok = candidate_has_more_recent_log || candidate_has_longer_log;

                // check to see if candidate's term is up to date with ours
                let up_to_date = req.candidate_term == self.current_term;

                // check to make sure we haven't voted yet (or we've already voted for them to make this idempotent)
                let havent_voted_for_them = match self.voted_for {
                    Some(voted_candidate_id) => voted_candidate_id == req.candidate_id,
                    None => true,
                };

                // construct a response depending on conditions
                let vote_granted = if log_ok && up_to_date && havent_voted_for_them {
                    // all conditions met! vote for them
                    self.voted_for = Some(req.candidate_id);
                    true
                } else {
                    false
                };
                let rpc = RPC::VoteResponse(VoteResponse {
                    votee_id: self.id,
                    term: self.current_term,
                    vote_granted,
                });
                let msg = (Target::Single(req.candidate_id), rpc);
                self.transport_layer.send(&msg).unwrap()
            }
            RPC::VoteResponse(res) => {
                if res.term > self.current_term {
                    // if votee is ahead, we are out of date, reset to follower
                    self.reset_to_follower(res.term);
                }

                let quorum = self.quorum_size();
                match &mut self.leadership_state {
                    Candidate(state) => {
                        let up_to_date = res.term == self.current_term;
                        // only process the vote if we are a candidate, the votee is voting for
                        // our current term, and the vote was positive
                        if up_to_date && res.vote_granted {
                            // add this to votes received
                            state.votes_received.insert(res.votee_id);

                            // check if we've hit quorum
                            if state.votes_received.len() >= quorum {
                                // success! promote self to leader
                                let mut followers = BTreeMap::new();

                                // initialize followers to all nodes who voted for us
                                // then replicate our logs to them
                                state.votes_received.iter().for_each(|votee| {
                                    // add that votee to our list of followers
                                    followers.insert(
                                        *votee,
                                        NodeReplicationState {
                                            sent_up_to: self.log.last_idx() + 1,
                                            acked_up_to: 0,
                                        },
                                    );

                                    // replicate our log with each follower
                                    let rpc = RPC::AppendRequest(AppendRequest {
                                        entries: self.log.entries.clone().into(),
                                        leader_id: self.id,
                                        leader_term: self.current_term,
                                        leader_commit: self.log.commit_idx,
                                        leader_last_log_idx: self.log.last_idx(),
                                        leader_last_log_term: self.log.last_term(),
                                    });
                                    let msg = (Target::Single(*votee), rpc);
                                    self.transport_layer.send(&msg).unwrap();
                                });

                                // setup done, set state to leader
                                self.leadership_state = Leader(LeaderState {
                                    followers,
                                    heartbeat_timeout: self.config.heartbeat_interval,
                                });
                            }
                        }
                    }
                    _ => {}
                }
            }
            RPC::AppendRequest(req) => {
                // TODO: stub
            }
            RPC::AppendResponse(res) => {
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