use crate::{
    debug::*,
    log::{App, Log, LogEntry, LogIndex},
    rpc::{AppendRequest, AppendResponse, SendableMessage, Target, VoteRequest, VoteResponse, RPC},
};
use anyhow::{bail, Result};
use colored::Colorize;
use rand::Rng;
use rand_chacha::ChaCha8Rng;
use rand_core::SeedableRng;
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Debug,
    ops::Div,
    vec,
};

/// Type alias for Raft leadership term
pub type Term = u64;

/// Type alias for the ID of a single Raft server
pub type ServerId = usize;

/// Type alias for a unit of logical time
type Ticks = u32;

/// Configuration options for a Raft server
#[derive(Clone)]
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
pub enum RaftLeadershipState {
    /// Issues no requests but responds to requests from leaders and candidates.
    /// All Raft Nodes start in Follower state
    Follower(FollowerState),

    /// Used to elect a new leader.
    Candidate(CandidateState),

    /// Handles all client requests.
    Leader(LeaderState),
}

pub struct FollowerState {
    /// Ticks left to start an election if not reset by activity/heartbeat
    election_time: Ticks,
    /// Current leader node is following
    leader: Option<ServerId>,
}

pub struct CandidateState {
    /// Ticks left to start an election if quorum is not reached
    election_time: Ticks,
    /// Set of all nodes this node has received votes for
    votes_received: BTreeSet<ServerId>,
}

pub struct LeaderState {
    /// Track state about followers to figure out what to send them next
    followers: BTreeMap<ServerId, NodeReplicationState>,
    /// Ticks left till when to send the next heartbeat
    heartbeat_timeout: Ticks,
}

/// State of a single Node as tracked by a leader
pub struct NodeReplicationState {
    /// Index of next log entry to send to that server.
    /// Initialized to leader's last log index + 1
    pub sent_up_to: LogIndex,

    /// Index of highest log entry known to be replicated on server.
    /// Initialized to 0, increases monotonically
    pub acked_up_to: LogIndex,
}

/// A Raft server that replicates Logs of type `T`
pub struct RaftServer<T, S> {
    // Static State
    /// ID of this node
    id: ServerId,
    /// All other servers in this Raft cluster
    peers: BTreeSet<ServerId>,
    /// Config of this node
    config: RaftConfig,

    // Persistent State
    // In a smarter implementation, these need to be persisted to disk
    // So we can recover these in case of a crash
    /// Current term of this node
    current_term: Term,
    /// Candidate node that we voted for this election
    voted_for: Option<ServerId>,
    /// List of log entries for this node.
    /// This is the data that is being replicated
    log: Log<T, S>,

    /// State of the node that depends on its leadership status
    /// (one of [`FollowerState`], [`CandidateState`], or [`LeaderState`])
    leadership_state: RaftLeadershipState,

    /// Internal seeded random number generator
    rng: ChaCha8Rng,
}

impl<T, S> RaftServer<T, S>
where
    T: Clone + Debug,
{
    pub fn new(
        id: ServerId,
        peers: BTreeSet<ServerId>,
        config: RaftConfig,
        seed: Option<u64>,
        app: Box<dyn App<T, S>>,
    ) -> Self {
        // Create RNG generator from seed if it exists, otherwise seed from system entropy
        let mut rng = match seed {
            Some(n) => ChaCha8Rng::seed_from_u64(n),
            None => ChaCha8Rng::from_entropy(),
        };
        let random_election_time = rng_jitter(
            &mut rng,
            config.election_timeout,
            config.election_timeout_jitter,
        );
        let server = RaftServer {
            id,
            peers,
            config,
            current_term: 0,
            voted_for: None,
            log: Log::new(id, app),
            rng,
            leadership_state: RaftLeadershipState::Follower(FollowerState {
                leader: None,
                election_time: random_election_time,
            }),
        };
        log(&id, "initializing server".to_owned(), Level::Overview);
        server.log_state_update();
        server
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
    pub fn tick(&mut self) -> Vec<SendableMessage<T>> {
        use RaftLeadershipState::*;
        match &mut self.leadership_state {
            Follower(FollowerState { election_time, .. })
            | Candidate(CandidateState { election_time, .. }) => {
                *election_time = election_time.saturating_sub(1);

                // suspect leader has failed, election timeout reached
                // attempt to become candidate
                if *election_time == 0 {
                    self.current_term += 1;
                    log(
                        &self.id,
                        format!(
                            "election timer expired, bumped to {} and started election",
                            colour_term(self.current_term)
                        ),
                        Level::Overview,
                    );

                    // vote for self
                    self.voted_for = Some(self.id);
                    let mut vote_list = BTreeSet::new();
                    vote_list.insert(self.id);

                    // see if we can instantly become leader
                    // (if cluster size is 1)
                    if 1 == self.quorum_size() {
                        log(
                            &self.id,
                            "cluster size is 1, trivially won election".to_owned(),
                            Level::Trace,
                        );
                        return self.promote_to_leader(BTreeMap::new());
                    }

                    // otherwise, become candidate as normal
                    self.leadership_state = Candidate(CandidateState {
                        election_time: self.random_election_time(),
                        votes_received: vote_list,
                    });
                    self.log_state_update();

                    // broadcast message to all nodes asking for a vote
                    let rpc = RPC::VoteRequest(VoteRequest {
                        candidate_term: self.current_term,
                        candidate_id: self.id,
                        candidate_last_log_idx: self.log.last_idx(),
                        candidate_last_log_term: self.log.last_term(),
                    });
                    return self.log_outgoing_rpcs(vec![(Target::Broadcast, rpc)]);
                }
            }
            Leader(state) => {
                state.heartbeat_timeout = state.heartbeat_timeout.saturating_sub(1);
                if state.heartbeat_timeout == 0 {
                    log(
                        &self.id,
                        "sending heartbeat to all followers".to_owned(),
                        Level::Trace,
                    );
                    let msgs = self.replicate_log(Target::Broadcast);
                    return self.log_outgoing_rpcs(msgs);
                }
            }
        }

        // fallthrough, no notable events, don't send anything
        vec![]
    }

    /// Helper function to reset current state back to follower if we are behind
    fn reset_to_follower(&mut self, new_term: Term) {
        if new_term > self.current_term {
            log(
                &self.id,
                format!(
                    "bumping our term {} to match candidate/leader term {}",
                    colour_term(self.current_term),
                    colour_term(new_term)
                ),
                Level::Trace,
            );
            self.current_term = new_term;
        }
        self.voted_for = None;
        if !self.is_follower() {
            self.log_state_update();
        }
        self.leadership_state = RaftLeadershipState::Follower(FollowerState {
            leader: None, // as we are in an election
            election_time: self.random_election_time(),
        });
    }

    /// Calculate quorum of current set of peers.
    /// quorum = ceil((peers.length + 1)/2)
    fn quorum_size(&self) -> usize {
        // add an extra because self.peers doesn't include self
        self.peers.len().saturating_add(2).div(2)
    }

    /// Demultiplex incoming RPC to its correct receiver function
    pub fn receive_rpc(&mut self, rpc: &RPC<T>) -> Vec<SendableMessage<T>> {
        log(&self.id, format!("<- {rpc}"), Level::Overview);
        let msgs = match rpc {
            RPC::VoteRequest(req) => self.rpc_vote_request(req),
            RPC::VoteResponse(res) => self.rpc_vote_response(res),
            RPC::AppendRequest(req) => self.rpc_append_request(req),
            RPC::AppendResponse(res) => self.rpc_append_response(res),
        };
        self.log_outgoing_rpcs(msgs)
    }

    pub fn client_request(&mut self, msg: T) -> Result<()> {
        log(
            &self.id,
            "received client_request to add an entry".to_owned(),
            Level::Overview,
        );
        match &mut self.leadership_state {
            RaftLeadershipState::Leader(_) => {
                // append log entry
                self.log.entries.push(LogEntry {
                    term: self.current_term,
                    data: msg,
                });

                // replicate our log to followers
                self.replicate_log(Target::Broadcast);
                Ok(())
            }
            _ => {
                // we aren't a leader so not authorized to add to the replicated log
                // respond to client by saying we are not the leader. client is responsible
                // for trying again with a different server
                bail!("cannot add a log entry to a non-leader!")

                // in a more robust implementation, client requests would generate a unique
                // serial number of each request (client id, request number) and 'retry' with
                // each peer until it succeeds. servers then track latest serial number for each
                // client plus associated response. on duplicates, the leader sends the old response with
                // re-executing the msg (linearizable)
            }
        }
    }

    /// Replicate some section of our log entries to followers.
    /// Intended to only be called when we are a Leader, do nothing otherwise
    fn replicate_log(&mut self, target: Target) -> Vec<SendableMessage<T>> {
        if let RaftLeadershipState::Leader(state) = &self.leadership_state {
            // construct closure for the sending logic so we don't need
            // to duplicate logic

            let sending_logic = |target| {
                // prefix len is the index of all the entries we have sent up to
                let prefix_len = state
                    .followers
                    .get(target)
                    .unwrap_or_else(|| panic!("target={} is not a follower", target))
                    .sent_up_to;
                let prefix_term = if self.log.entries.len() > 0 {
                    self.log.entries.get(prefix_len - 1).unwrap().term
                } else {
                    0
                };
                let entries = self.log.entries[prefix_len..self.log.entries.len()].to_vec();

                if entries.len() == 0 {
                    log(
                        &self.id,
                        format!("preparing heartbeat signal to {}", colour_server(target)),
                        Level::Trace,
                    );
                } else {
                    log(
                        &self.id,
                        format!(
                            "preparing RPC call to {}... replicating a portion of our log: {}",
                            colour_server(target),
                            debug_log(
                                &self.log.entries,
                                vec![(
                                    AnnotationType::Span(prefix_len, self.log.entries.len()),
                                    "these entries"
                                )],
                                0
                            ),
                        ),
                        Level::Trace,
                    );
                }

                let rpc = RPC::AppendRequest(AppendRequest {
                    entries,
                    leader_id: self.id,
                    leader_term: self.current_term,
                    leader_commit: self.log.committed_len,
                    leader_last_log_idx: prefix_len,
                    leader_last_log_term: prefix_term,
                });
                (Target::Single(*target), rpc)
            };

            match target {
                Target::Single(target) => vec![sending_logic(&target)],
                Target::Broadcast => state.followers.keys().map(sending_logic).collect(),
            }
        } else {
            vec![]
        }
    }

    /// Process an RPC Request to vote for requesting candidate
    fn rpc_vote_request(&mut self, req: &VoteRequest) -> Vec<SendableMessage<T>> {
        log(
            &self.id,
            format!(
                "[rpc_vote_request] from {}",
                colour_server(&req.candidate_id)
            ),
            Level::Requests,
        );

        if req.candidate_term > self.current_term {
            // if we are behind the other candidate, just reset to follower
            self.reset_to_follower(req.candidate_term);
        }

        // check if candidate's log is up to date with ours
        // if they are outdated, don't vote for them (we don't want an outdated leader)
        let candidate_has_more_recent_log = req.candidate_last_log_term > self.log.last_term();
        let candidate_has_longer_log = req.candidate_last_log_term == self.log.last_term()
            && req.candidate_last_log_idx >= self.log.last_idx();
        let log_ok = candidate_has_more_recent_log || candidate_has_longer_log;

        // check to see if candidate's term is up to date with ours
        let up_to_date = req.candidate_term == self.current_term;

        // check to make sure we haven't voted yet (or we've already voted for them to make this idempotent)
        let havent_voted = match self.voted_for {
            Some(voted_candidate_id) => voted_candidate_id == req.candidate_id,
            None => true,
        };

        // construct a response depending on conditions
        let vote_granted = if log_ok && up_to_date && havent_voted {
            // all conditions met! vote for them
            self.voted_for = Some(req.candidate_id);
            true
        } else {
            false
        };
        log(
            &self.id,
            format!(
                "vote: {} because\n1) their log has a more recent term or is longer: {}\n2) their term is up to date: {}\n3) we haven't voted this election cycle or we already voted for them: {}",
                colour_bool(vote_granted),
                colour_bool(log_ok),
                colour_bool(up_to_date),
                colour_bool(havent_voted)
            ),
            Level::Trace,
        );
        let rpc = RPC::VoteResponse(VoteResponse {
            votee_id: self.id,
            term: self.current_term,
            vote_granted,
        });
        vec![(Target::Single(req.candidate_id), rpc)]
    }

    /// Process an RPC response to [`rpc_vote_request`]
    fn rpc_vote_response(&mut self, res: &VoteResponse) -> Vec<SendableMessage<T>> {
        log(
            &self.id,
            format!(
                "[rpc_vote_response] from {} voting {}",
                colour_server(&res.votee_id),
                colour_bool(res.vote_granted)
            ),
            Level::Requests,
        );
        if res.term > self.current_term {
            // if votee is ahead, we are out of date, reset to follower
            self.reset_to_follower(res.term);
        }

        let quorum = self.quorum_size();
        if let RaftLeadershipState::Candidate(state) = &mut self.leadership_state {
            let up_to_date = res.term == self.current_term;
            // only process the vote if we are a candidate, the votee is voting for
            // our current term, and the vote was positive
            log(
                &self.id,
                format!(
                    "counting vote: {} because\n1) follower is up to date: {}\n2) they granted the vote: {}",
                    colour_bool(up_to_date && res.vote_granted),
                    colour_bool(up_to_date),
                    colour_bool(res.vote_granted),
                ),
                Level::Trace,
            );
            if up_to_date && res.vote_granted {
                // add this to votes received
                state.votes_received.insert(res.votee_id);
                log(
                    &self.id,
                    format!(
                        "total vote count: {} out of quorum of {}",
                        state.votes_received.len(),
                        quorum,
                    ),
                    Level::Trace,
                );

                if state.votes_received.len() < quorum {
                    return vec![];
                }

                // otherwise, we won election! promote self to leader
                // initialize followers to all nodes who voted for us
                let mut followers = BTreeMap::new();
                self.peers
                    .iter()
                    .filter(|votee| **votee != self.id)
                    .for_each(|votee| {
                        // add that votee to our list of followers
                        match followers.insert(
                            *votee,
                            NodeReplicationState {
                                sent_up_to: self.log.last_idx(),
                                acked_up_to: 0,
                            },
                        ) {
                            None => log(
                                &self.id,
                                format!("added {} to list of followers", colour_server(votee)),
                                Level::Trace,
                            ),
                            _ => {}
                        };
                    });
                return self.promote_to_leader(followers);
            }
        }

        // fallthrough case, do nothing
        vec![]
    }

    fn promote_to_leader(
        &mut self,
        followers: BTreeMap<ServerId, NodeReplicationState>,
    ) -> Vec<SendableMessage<T>> {
        let num_votes = followers.len() + 1;
        let follower_ids: Vec<ServerId> = followers.keys().cloned().collect();

        // set state to leader
        self.leadership_state = RaftLeadershipState::Leader(LeaderState {
            followers,
            heartbeat_timeout: self.config.heartbeat_interval,
        });
        self.log_state_update();
        log(
            &self.id,
            format!(
                "won election with votes={} out of quorum={}, followers: {}",
                num_votes,
                self.quorum_size(),
                follower_ids
                    .iter()
                    .map(|id| colour_server(id))
                    .collect::<Vec<String>>()
                    .join(", ")
            ),
            Level::Requests,
        );

        // then replicate our logs to all our followers
        self.replicate_log(Target::Broadcast)
    }

    /// Process an RPC request to append a message to the replicated event log
    fn rpc_append_request(&mut self, req: &AppendRequest<T>) -> Vec<SendableMessage<T>> {
        log(
            &self.id,
            format!(
                "[rpc_append_request] from {}",
                colour_server(&req.leader_id),
            ),
            Level::Requests,
        );

        // check to see if we are out of date
        if req.leader_term > self.current_term {
            self.reset_to_follower(req.leader_term);
        }

        match &mut self.leadership_state {
            RaftLeadershipState::Candidate(_) | RaftLeadershipState::Leader(_) => {
                // if leader is in same term as us, they have recovered from
                // failure and we can go back to follower and try the request again
                log(
                    &self.id,
                    format!(
                        "comparing our term {} to supposed leader {}",
                        colour_term(self.current_term),
                        colour_term(req.leader_term)
                    ),
                    Level::Trace,
                );

                if req.leader_term == self.current_term {
                    log(
                        &self.id,
                        format!("term matches, reset to follower, update term and retry"),
                        Level::Trace,
                    );

                    self.reset_to_follower(req.leader_term);
                    self.rpc_append_request(req)
                } else {
                    // otherwise just do nothing, only followers should response to
                    // append_request RPCs
                    log(&self.id, format!("outdated, ignoring"), Level::Trace);
                    vec![]
                }
            }
            RaftLeadershipState::Follower(state) => {
                // if leader is same term as us, we accept requester as current leader
                log(
                    &self.id,
                    format!(
                        "checking pre-req, does our term match the leader's term: {}",
                        colour_bool(req.leader_term == self.current_term),
                    ),
                    Level::Trace,
                );
                let success = if req.leader_term == self.current_term {
                    state.leader = Some(req.leader_id);

                    // check if we have the messages that the leader is claiming we have
                    let prefix_len = req.leader_last_log_idx;
                    let prefix_ok = self.log.entries.len() >= prefix_len;
                    let last_log_entry_matches_terms = prefix_len == 0
                        || (self
                            .log
                            .entries
                            .get(prefix_len - 1)
                            .expect("invalid leader_last_log_idx")
                            .term
                            == req.leader_last_log_term);

                    log(&self.id, format!(
                        "append entries: {} because\n1) the index they want to insert entries at ({}) <= our log length ({}): {}\n2) last log entry before new entries matches terms: {}",
                        colour_bool(prefix_ok && last_log_entry_matches_terms),
                        prefix_len,
                        self.log.entries.len(),
                        colour_bool(prefix_ok),
                        colour_bool(last_log_entry_matches_terms)
                    ), Level::Trace);
                    if prefix_ok && last_log_entry_matches_terms {
                        // assumptions match, append it to our local log
                        self.log
                            .append_entries(prefix_len, req.leader_commit, req.entries.clone());
                        true // success
                    } else {
                        false // bad request if we have mismatched assumptions about where the log is
                    }
                } else {
                    false // bad request if we have mismatched terms
                };

                // send response
                let rpc = RPC::AppendResponse(AppendResponse {
                    ok: success,
                    term: self.current_term,
                    ack_idx: self.log.entries.len(),
                    follower_id: self.id,
                });
                vec![(Target::Single(req.leader_id), rpc)]
            }
        }
    }

    /// Process an RPC response to [`rpc_append_request`]
    fn rpc_append_response(&mut self, res: &AppendResponse) -> Vec<SendableMessage<T>> {
        log(
            &self.id,
            format!(
                "[rpc_append_response] from {}",
                colour_server(&res.follower_id),
            ),
            Level::Requests,
        );

        // check to see if we are out of date
        if res.term > self.current_term {
            self.reset_to_follower(res.term);
        }

        if let RaftLeadershipState::Leader(state) = &mut self.leadership_state {
            if res.term == self.current_term {
                // make sure that the response was ok and the length that the follower is
                // at is greater than what we have recorded for them before
                let follower_state = state
                    .followers
                    .get_mut(&res.follower_id)
                    .expect("unknown/invalid follower id");
                log(
            &self.id,
            format!(
                "valid response: {} because\n1) response indicated success: {}\n2) the index they acked up to (new={}) actually moved forward (old={}): {}",
                colour_bool(res.ok && res.ack_idx >= follower_state.acked_up_to),
                colour_bool(res.ok),
                res.ack_idx,
                follower_state.acked_up_to,
                res.ack_idx >= follower_state.acked_up_to,
            ),
            Level::Trace,
        );
                if res.ok && res.ack_idx >= follower_state.acked_up_to {
                    // update replication state, we know follower has sent + acked up
                    // to `replication_state.ack_idx`
                    format!(
                        "success! bumping sent_up_to from {} -> {} and acked_up_to from {} -> {}",
                        follower_state.sent_up_to,
                        res.ack_idx,
                        follower_state.acked_up_to,
                        res.ack_idx
                    );
                    follower_state.sent_up_to = res.ack_idx;
                    follower_state.acked_up_to = res.ack_idx;
                    // try to formally commit these entries, no need to respond
                    self.commit_log_entries();
                    return vec![];
                } else if follower_state.sent_up_to > 0 {
                    // if there's a gap in the log, res.ok is not true!
                    // reduce what we assume the client has received by one and try again
                    format!(
                        "error, decrement sent_up_to from {} -> {} and try again",
                        follower_state.sent_up_to,
                        follower_state.sent_up_to - 1,
                    );
                    follower_state.sent_up_to = follower_state.sent_up_to.saturating_sub(1);
                    return self.replicate_log(Target::Single(res.follower_id));
                } else {
                    // something is critically wrong
                    panic!("invalid append_response received: already tried resending whole log and response still fails");
                }
            } else {
                // this should never be reached, client should have updated their term when we sent the first response
                panic!("invalid append_response received: client term should never be behind at this point");
            }
        } else {
            vec![]
        }
    }

    /// Commit any log entries that have been acknowledged by a quorum of nodes.
    /// When a log entry is committed, its message is delivered to the application.
    fn commit_log_entries(&mut self) {
        let quorum_size = self.quorum_size();
        match &mut self.leadership_state {
            RaftLeadershipState::Leader(state) => {
                // construct a collection of all nodes in system
                let mut all_nodes: Vec<&ServerId> = self.peers.iter().collect();
                all_nodes.push(&self.id);

                // repeat until we have committed all entries
                while self.log.committed_len < self.log.entries.len() {
                    // count all nodes which have acked past what our current commit_len is
                    // +1 is to include ourselves!
                    let acks = state
                        .followers
                        .values()
                        .filter(|follower_state| {
                            follower_state.acked_up_to > self.log.committed_len
                        })
                        .count()
                        + 1;

                    format!(
                        "trying to commit entry at index ({}): {} because\n1) total of {} acks for that index >= quorum size ({})",
                        self.log.committed_len,
                        colour_bool(acks >= quorum_size),
                        acks,
                        quorum_size,
                    );

                    if acks >= quorum_size {
                        // hit quorum! deliver last log to application and bump commit_len
                        self.log.deliver_msg();
                        self.log.committed_len += 1;
                    } else {
                        // exit early, nothing we can do except wait for more nodes to acknowledge
                        // the entries we told them to add
                        break;
                    }
                }
            }
            _ => {}
        }
    }

    /// Logging helpers

    pub fn is_leader(&self) -> bool {
        match self.leadership_state {
            RaftLeadershipState::Leader(_) => true,
            _ => false,
        }
    }

    pub fn is_candidate(&self) -> bool {
        match self.leadership_state {
            RaftLeadershipState::Candidate(_) => true,
            _ => false,
        }
    }

    pub fn is_follower(&self) -> bool {
        match self.leadership_state {
            RaftLeadershipState::Follower(_) => true,
            _ => false,
        }
    }

    fn log_state_update(&self) {
        let state_str = match self.leadership_state {
            RaftLeadershipState::Leader(_) => " Leader ".on_blue(),
            RaftLeadershipState::Candidate(_) => " Candidate ".on_yellow(),
            RaftLeadershipState::Follower(_) => " Follower ".on_truecolor(140, 140, 140),
        }
        .bold()
        .black()
        .to_string();
        log(&self.id, format!("is now {}", state_str), Level::Overview);
    }

    fn log_outgoing_rpcs(&self, msgs: Vec<SendableMessage<T>>) -> Vec<SendableMessage<T>> {
        msgs.iter().for_each(|msg| {
            log(
                &self.id,
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
}

/// Returns a random u32 uniformly from (expected)
fn rng_jitter(rng: &mut ChaCha8Rng, expected: u32, jitter: u32) -> u32 {
    let low = expected - jitter;
    let hi = expected + jitter;
    rng.gen_range(low..=hi)
}
