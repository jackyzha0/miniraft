//! Antithesis-style deterministic simulation testing for miniraft.
//!
//! Methodology follows the Antithesis post on finding bugs in Raft
//! implementations (https://antithesis.com/blog/2026/finding-bugs-in-raft-implementations/):
//!
//! 1. Run a trivially simple workload — a "chain of blocks" state machine whose
//!    state is `(number of commands applied, running hash)`. If two nodes ever
//!    disagree on the hash at the same applied-count, they applied different
//!    commands (or the same commands in a different order): a State Machine
//!    Safety violation.
//! 2. Subject the cluster to aggressive fault injection: message delay,
//!    reordering, loss, duplication, network partitions, and node pauses.
//! 3. Continuously check Raft's safety properties as `always` assertions, and
//!    check that the workload is actually exercising the system with
//!    `sometimes` assertions.
//! 4. After fault injection stops, check the "eventually" properties: a single
//!    leader emerges, new commands commit, and all nodes converge.
//!
//! The whole cluster is simulated in-process with seeded RNGs, so every run is
//! reproducible: a failing seed is a repro. Locally we sweep seeds; under
//! Antithesis proper the platform supplies the entropy and explores the state
//! space for us (the assertions below use the real Antithesis SDK, so they
//! would be picked up as test properties).
//!
//! Repro a specific failure with:
//!   MINIRAFT_SIM_SEED=<seed> cargo test --test antithesis -- --nocapture
//! (add RUST_LOG=trace for a full event trace)

use antithesis_sdk::{antithesis_init, assert_always, assert_sometimes};
use miniraft::log::{App, LogEntry};
use miniraft::rpc::{SendableMessage, Target, RPC};
use miniraft::server::{RaftConfig, RaftServer, ServerId, Term};
use rand::{Rng, RngCore, SeedableRng};
use rand_chacha::ChaCha8Rng;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::panic::{self, AssertUnwindSafe};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

/// Commands are unique u64s so that any divergence in application order is
/// visible in the hash chain.
type Cmd = u64;
/// (number of commands applied, running hash)
type ChainState = (u64, u64);

/// The "chain of blocks" app from the blog post: each applied command is
/// folded into a running hash. Divergent histories can never re-converge to
/// the same hash, so comparing `(count, hash)` across nodes detects any
/// violation of total-order delivery.
struct ChainApp {
    count: u64,
    hash: u64,
}

impl App<Cmd, ChainState> for ChainApp {
    fn transition_fn(&mut self, entry: &LogEntry<Cmd>) {
        self.count += 1;
        self.hash = mix(self.hash, entry.term, entry.data);
    }
    fn get_state(&self) -> ChainState {
        (self.count, self.hash)
    }
}

/// splitmix64-style mixer folding (term, data) into the previous hash
fn mix(prev: u64, term: Term, data: Cmd) -> u64 {
    let mut x = prev
        ^ term.wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ data.rotate_left(32);
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^= x >> 31;
    x
}

const CFG: RaftConfig = RaftConfig {
    election_timeout: 10,
    election_timeout_jitter: 3,
    heartbeat_interval: 5,
};

/// Ticks spent under fault injection
const FAULT_TICKS: u64 = 1500;
/// Ticks of fault-free operation to let the cluster settle
const QUIESCE_TICKS: u64 = 500;
/// Ticks to wait for the post-quiesce workload to commit everywhere
const CONVERGE_TICKS: u64 = 300;

/// Per-message probability of being dropped
const P_DROP: f64 = 0.05;
/// Per-message probability of being duplicated
const P_DUP: f64 = 0.03;
/// Per-message probability of a long delay (causes reordering across terms)
const P_LONG_DELAY: f64 = 0.03;
/// Per-tick probability of changing the network partition
const P_REPARTITION: f64 = 0.02;
/// Per-tick probability of pausing/resuming a random node
const P_PAUSE_TOGGLE: f64 = 0.01;
/// Per-tick probability of pausing a current leader (targeted fault)
const P_LEADER_PAUSE: f64 = 0.01;
/// Per-tick, per-leader probability of submitting a client command
const P_CLIENT: f64 = 0.25;

/// Tick counter exposed so panics from inside miniraft can be attributed
static CURRENT_TICK: AtomicU64 = AtomicU64::new(0);

thread_local! {
    /// MINIRAFT_SIM_PROBE=lo:hi prints message flow for that tick range
    static PROBE_RANGE: Option<(u64, u64)> = std::env::var("MINIRAFT_SIM_PROBE")
        .ok()
        .and_then(|s| {
            let (lo, hi) = s.split_once(':')?;
            Some((lo.parse().ok()?, hi.parse().ok()?))
        });
}
/// (message, location) of the most recent panic, captured by our hook
static LAST_PANIC: Mutex<Option<(String, String)>> = Mutex::new(None);

/// A message in flight on the simulated network
struct InFlight {
    deliver_at: u64,
    /// tie-breaker so equal-delay messages keep FIFO order (determinism)
    seq: u64,
    from: ServerId,
    to: ServerId,
    /// Rc because broadcast and duplication deliver the same RPC repeatedly
    rpc: Rc<RPC<Cmd>>,
}

#[derive(Default, Clone)]
struct Stats {
    max_term: Term,
    max_committed: usize,
    truncations: u64,
    cmds_submitted: u64,
}

struct Sim {
    seed: u64,
    rng: ChaCha8Rng,
    now: u64,
    servers: BTreeMap<ServerId, RaftServer<Cmd, ChainState>>,
    net: Vec<InFlight>,
    seq: u64,
    /// current partition: links crossing the set boundary are cut
    partition: Option<BTreeSet<ServerId>>,
    paused: BTreeSet<ServerId>,
    faults_enabled: bool,
    next_cmd: Cmd,

    // ---- oracle state (ground truth accumulated across the whole run) ----
    /// Election Safety: term -> the unique leader we saw for it
    leaders_by_term: BTreeMap<Term, ServerId>,
    /// Durability: log index -> (term, data) of the first entry ever
    /// observed as committed there. May never change afterwards.
    committed: BTreeMap<usize, (Term, Cmd)>,
    /// State Machine Safety: applied-count -> chain hash. All nodes passing
    /// through the same count must have the same hash.
    chain: BTreeMap<u64, u64>,
    /// previous log length per server, to observe rollbacks
    prev_len: BTreeMap<ServerId, usize>,

    stats: Stats,
}

impl Sim {
    fn new(seed: u64, n: usize) -> Self {
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let ids: BTreeSet<ServerId> = (0..n).collect();
        let mut servers = BTreeMap::new();
        for id in &ids {
            let mut peers = ids.clone();
            peers.remove(id);
            servers.insert(
                *id,
                RaftServer::new(
                    *id,
                    peers,
                    CFG.clone(),
                    Some(rng.next_u64()),
                    Box::new(ChainApp { count: 0, hash: 0 }),
                ),
            );
        }
        Sim {
            seed,
            rng,
            now: 0,
            servers,
            net: Vec::new(),
            seq: 0,
            partition: None,
            paused: BTreeSet::new(),
            faults_enabled: true,
            next_cmd: 1,
            leaders_by_term: BTreeMap::new(),
            committed: BTreeMap::new(),
            chain: BTreeMap::new(),
            prev_len: BTreeMap::new(),
            stats: Stats::default(),
        }
    }

    fn link_blocked(&self, from: ServerId, to: ServerId) -> bool {
        match &self.partition {
            Some(group) => group.contains(&from) != group.contains(&to),
            None => false,
        }
    }

    fn sample_delay(&mut self) -> u64 {
        if self.faults_enabled {
            if self.rng.gen_bool(P_LONG_DELAY) {
                self.rng.gen_range(10..=40)
            } else {
                self.rng.gen_range(1..=3)
            }
        } else {
            1
        }
    }

    /// Enqueue outgoing messages, applying drop/duplicate/delay faults
    fn send(&mut self, from: ServerId, msgs: Vec<SendableMessage<Cmd>>) {
        for (target, rpc) in msgs {
            let rpc = Rc::new(rpc);
            let targets: Vec<ServerId> = match target {
                Target::Single(t) => vec![t],
                Target::Broadcast => {
                    self.servers.keys().copied().filter(|id| *id != from).collect()
                }
            };
            for to in targets {
                if self.faults_enabled && self.rng.gen_bool(P_DROP) {
                    continue;
                }
                let copies = if self.faults_enabled && self.rng.gen_bool(P_DUP) {
                    2
                } else {
                    1
                };
                for _ in 0..copies {
                    let delay = self.sample_delay();
                    self.net.push(InFlight {
                        deliver_at: self.now + delay,
                        seq: self.seq,
                        from,
                        to,
                        rpc: Rc::clone(&rpc),
                    });
                    self.seq += 1;
                }
            }
        }
    }

    /// Randomly evolve partitions and paused nodes
    fn inject_faults(&mut self) {
        if !self.faults_enabled {
            return;
        }
        if self.rng.gen_bool(P_REPARTITION) {
            if self.rng.gen_bool(0.3) {
                self.partition = None; // heal
            } else {
                let group: BTreeSet<ServerId> = {
                    let ids: Vec<ServerId> = self.servers.keys().copied().collect();
                    ids.into_iter().filter(|_| self.rng.gen_bool(0.5)).collect()
                };
                self.partition = Some(group);
            }
        }
        if self.rng.gen_bool(P_PAUSE_TOGGLE) {
            let ids: Vec<ServerId> = self.servers.keys().copied().collect();
            let pick = ids[self.rng.gen_range(0..ids.len())];
            if !self.paused.remove(&pick) {
                self.paused.insert(pick);
            }
        }
        // targeted fault: pausing a leader right after it accepts entries is
        // what sets up figure-8 style histories (partially replicated entries
        // from old terms), so aim there specifically
        if self.rng.gen_bool(P_LEADER_PAUSE) {
            let leaders: Vec<ServerId> = self
                .servers
                .values()
                .filter(|s| s.is_leader())
                .map(|s| s.id)
                .collect();
            if !leaders.is_empty() {
                let pick = leaders[self.rng.gen_range(0..leaders.len())];
                self.paused.insert(pick);
            }
        }
    }

    /// A stateless client: submit a fresh unique command to every node that
    /// currently believes it is the leader (there may be several!).
    /// Stops with fault injection so the "eventually" convergence checks run
    /// against a quiescent system.
    fn client_workload(&mut self) {
        if !self.faults_enabled {
            return;
        }
        let leaders: Vec<ServerId> = self
            .servers
            .values()
            .filter(|s| s.is_leader() && !self.paused.contains(&s.id))
            .map(|s| s.id)
            .collect();
        for id in leaders {
            if self.rng.gen_bool(P_CLIENT) {
                let cmd = self.next_cmd;
                self.next_cmd += 1;
                let _ = self.servers.get_mut(&id).unwrap().client_request(cmd);
                self.stats.cmds_submitted += 1;
            }
        }
    }

    fn tick(&mut self) {
        self.now += 1;
        CURRENT_TICK.store(self.now, Ordering::Relaxed);
        self.inject_faults();

        // tick every running node
        let ids: Vec<ServerId> = self.servers.keys().copied().collect();
        for id in &ids {
            if self.paused.contains(id) {
                continue;
            }
            let msgs = self.servers.get_mut(id).unwrap().tick();
            self.send(*id, msgs);
        }

        self.client_workload();

        // deliver everything that is due this tick (responses generated during
        // delivery are enqueued with delay >= 1, so they land on later ticks)
        let (due, rest): (Vec<InFlight>, Vec<InFlight>) = self
            .net
            .drain(..)
            .partition(|m| m.deliver_at <= self.now);
        self.net = rest;
        let mut due = due;
        due.sort_by_key(|m| (m.deliver_at, m.seq));
        let probing = PROBE_RANGE.with(|r| {
            r.map(|(lo, hi)| self.now >= lo && self.now <= hi).unwrap_or(false)
        });
        for m in due {
            // partitions and pauses drop packets at delivery time
            if self.paused.contains(&m.to) || self.link_blocked(m.from, m.to) {
                continue;
            }
            let responses = self.servers.get_mut(&m.to).unwrap().receive_rpc(&m.rpc);
            if probing {
                println!(
                    "t={} {}->{} {} => {} response(s)",
                    self.now, m.from, m.to, m.rpc, responses.len()
                );
            }
            self.send(m.to, responses);
        }
        if probing {
            for s in self.servers.values() {
                println!(
                    "t={}   server {} [{}] term={} len={} committed={}",
                    self.now,
                    s.id,
                    if s.is_leader() { "L" } else { "-" },
                    s.current_term,
                    s.log.entries.len(),
                    s.log.committed_len
                );
            }
        }

        self.check_invariants();
    }

    fn tick_by(&mut self, n: u64) {
        for _ in 0..n {
            self.tick();
        }
    }

    /// The `always` properties, checked against ground truth every tick
    fn check_invariants(&mut self) {
        let ids: Vec<ServerId> = self.servers.keys().copied().collect();

        // --- Election Safety: at most one leader per term ---
        for id in &ids {
            let s = self.servers.get(id).unwrap();
            self.stats.max_term = self.stats.max_term.max(s.current_term);
            if !s.is_leader() {
                continue;
            }
            let (term, sid) = (s.current_term, s.id);
            let prev = *self.leaders_by_term.entry(term).or_insert(sid);
            let ok = prev == sid;
            assert_always!(
                ok,
                "election safety: at most one leader per term",
                &json!({"term": term, "leader_a": prev, "leader_b": sid, "seed": self.seed, "tick": self.now})
            );
            if !ok {
                panic!(
                    "SAFETY (election safety): servers {} and {} are both leaders in term {}",
                    prev, sid, term
                );
            }
        }

        // --- Committed entries are durable and agreed upon ---
        for id in &ids {
            let s = self.servers.get(id).unwrap();
            let clen = s.log.committed_len;
            if clen > s.log.entries.len() {
                panic!(
                    "SAFETY (commit durability): server {} committed_len {} exceeds log length {}",
                    id, clen, s.log.entries.len()
                );
            }
            for i in 0..clen {
                let e = &s.log.entries[i];
                let observed = (e.term, e.data);
                match self.committed.get(&i) {
                    Some(first) => {
                        let ok = *first == observed;
                        assert_always!(
                            ok,
                            "commit durability: a committed entry never changes",
                            &json!({"index": i, "first": first, "now": observed, "server": id, "seed": self.seed, "tick": self.now})
                        );
                        if !ok {
                            panic!(
                                "SAFETY (commit durability): index {} was committed as {:?} but server {} has committed {:?}",
                                i, first, id, observed
                            );
                        }
                    }
                    None => {
                        self.committed.insert(i, observed);
                    }
                }
            }
            self.stats.max_committed = self.stats.max_committed.max(clen);
        }

        // --- State Machine Safety via the hash chain ---
        for id in &ids {
            let s = self.servers.get(id).unwrap();
            let (count, hash) = s.log.app.get_state();
            match self.chain.get(&count) {
                Some(first) => {
                    let ok = *first == hash;
                    assert_always!(
                        ok,
                        "state machine safety: identical state at identical applied count",
                        &json!({"applied": count, "hash_first": first, "hash_now": hash, "server": id, "seed": self.seed, "tick": self.now})
                    );
                    if !ok {
                        panic!(
                            "SAFETY (state machine safety): server {} applied {} commands and reached hash {:#x}, but another node reached {:#x} at the same count",
                            id, count, hash, first
                        );
                    }
                }
                None => {
                    self.chain.insert(count, hash);
                }
            }
        }

        // --- observe log rollbacks (for the `sometimes` workload check) ---
        for id in &ids {
            let len = self.servers.get(id).unwrap().log.entries.len();
            let prev = self.prev_len.insert(*id, len).unwrap_or(0);
            if len < prev {
                self.stats.truncations += 1;
            }
        }

        // --- Log Matching (pairwise, every 10 ticks: it's the pricey one) ---
        // If two logs contain an entry with the same index and term, the
        // entries must be identical. Commands are globally unique, so a data
        // mismatch here is definitive.
        if self.now % 10 == 0 {
            for (ai, a_id) in ids.iter().enumerate() {
                for b_id in ids.iter().skip(ai + 1) {
                    let a = &self.servers.get(a_id).unwrap().log.entries;
                    let b = &self.servers.get(b_id).unwrap().log.entries;
                    for i in 0..a.len().min(b.len()) {
                        if a[i].term == b[i].term {
                            let ok = a[i].data == b[i].data;
                            assert_always!(
                                ok,
                                "log matching: same index and term implies same entry",
                                &json!({"index": i, "term": a[i].term, "server_a": a_id, "server_b": b_id, "seed": self.seed, "tick": self.now})
                            );
                            if !ok {
                                panic!(
                                    "SAFETY (log matching): servers {} and {} disagree at index {} term {}: {:?} vs {:?}",
                                    a_id, b_id, i, a[i].term, a[i].data, b[i].data
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    /// leader in the highest term, if any
    fn current_leader(&self) -> Option<ServerId> {
        self.servers
            .values()
            .filter(|s| s.is_leader())
            .max_by_key(|s| s.current_term)
            .map(|s| s.id)
    }
}

/// Run the full scenario for one seed. Returns workload stats, or panics on
/// the first property violation / crash inside miniraft.
fn run_scenario(seed: u64) -> Stats {
    // odd seeds get a 5-node cluster, even seeds a 3-node cluster
    let n = if seed % 2 == 0 { 3 } else { 5 };
    let mut sim = Sim::new(seed, n);

    // phase 1: aggressive fault injection
    sim.tick_by(FAULT_TICKS);

    // `sometimes` properties: prove the workload exercised the system.
    // (Under Antithesis these must each fire in at least one run; locally we
    // aggregate them across seeds in the sweep report.)
    assert_sometimes!(
        sim.stats.max_committed > 0,
        "workload: entries commit despite fault injection",
        &json!({"seed": seed, "committed": sim.stats.max_committed})
    );
    assert_sometimes!(
        sim.stats.max_term > 1,
        "workload: repeated elections occur",
        &json!({"seed": seed, "max_term": sim.stats.max_term})
    );
    assert_sometimes!(
        sim.stats.truncations > 0,
        "workload: follower log rollback occurs",
        &json!({"seed": seed, "truncations": sim.stats.truncations})
    );

    // phase 2: heal everything and let the cluster settle
    sim.faults_enabled = false;
    sim.partition = None;
    sim.paused.clear();
    sim.tick_by(QUIESCE_TICKS);

    // "eventually" property 1: a single leader emerges
    let leaders: Vec<ServerId> = sim
        .servers
        .values()
        .filter(|s| s.is_leader())
        .map(|s| s.id)
        .collect();
    let one_leader = leaders.len() == 1;
    assert_always!(
        one_leader,
        "eventually: exactly one leader after faults stop",
        &json!({"leaders": leaders, "seed": seed})
    );
    if !one_leader {
        panic!(
            "LIVENESS (leader election): expected exactly one leader after {} healthy ticks, found {:?}",
            QUIESCE_TICKS, leaders
        );
    }

    // "eventually" property 2: the cluster still accepts and commits commands
    let base_count = {
        let lead = sim.servers.get(&sim.current_leader().unwrap()).unwrap();
        lead.log.app.get_state().0
    };
    for _ in 0..5 {
        let lead_id = sim.current_leader().expect("leader vanished during healthy operation");
        let cmd = sim.next_cmd;
        sim.next_cmd += 1;
        sim.servers
            .get_mut(&lead_id)
            .unwrap()
            .client_request(cmd)
            .expect("leader rejected client request during healthy operation");
        sim.tick_by(5);
    }
    sim.tick_by(CONVERGE_TICKS);

    // "eventually" property 3: all nodes converge on the same state
    let states: BTreeSet<ChainState> = sim
        .servers
        .values()
        .map(|s| s.log.app.get_state())
        .collect();
    let converged = states.len() == 1;
    assert_always!(
        converged,
        "eventually: all nodes converge to identical state",
        &json!({"states": states, "seed": seed})
    );
    if !converged {
        let detail: Vec<String> = sim
            .servers
            .values()
            .map(|s| {
                let tail_from = s.log.entries.len().saturating_sub(4);
                let tail: Vec<String> = s.log.entries[tail_from..]
                    .iter()
                    .enumerate()
                    .map(|(i, e)| format!("[{}]=(t{},{})", tail_from + i, e.term, e.data))
                    .collect();
                format!(
                    "server {} [{}] term={} log_len={} committed={} applied={:?} tail: {}",
                    s.id,
                    if s.is_leader() { "leader" } else { "-" },
                    s.current_term,
                    s.log.entries.len(),
                    s.log.committed_len,
                    s.log.app.get_state(),
                    tail.join(" ")
                )
            })
            .collect();
        panic!(
            "LIVENESS (convergence): nodes did not converge after faults stopped:\n  {}",
            detail.join("\n  ")
        );
    }
    let final_count = states.iter().next().unwrap().0;
    if final_count < base_count + 5 {
        panic!(
            "LIVENESS (commit progress): only {} of 5 post-fault commands were applied",
            final_count.saturating_sub(base_count)
        );
    }

    sim.stats
}

/// Collapse a panic message into a stable signature so identical bugs found
/// under different seeds group together in the report.
fn signature(msg: &str, location: &str) -> String {
    let first_line = msg.lines().next().unwrap_or("");
    let normalized: String = first_line
        .chars()
        .map(|c| if c.is_ascii_digit() { '#' } else { c })
        .take(140)
        .collect();
    format!("{} @ {}", normalized, location)
}

#[test]
fn antithesis_style_simulation() {
    antithesis_init();

    // MINIRAFT_SIM_SEED=n runs a single seed (repro mode);
    // MINIRAFT_SIM_SEEDS=n controls sweep width
    let seeds: Vec<u64> = match std::env::var("MINIRAFT_SIM_SEED") {
        Ok(s) => vec![s.parse().expect("MINIRAFT_SIM_SEED must be a u64")],
        Err(_) => {
            let n: u64 = std::env::var("MINIRAFT_SIM_SEEDS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(100);
            (0..n).collect()
        }
    };

    // capture panic messages + locations quietly; the report at the end is
    // the interesting output, not 100 backtraces
    let old_hook = panic::take_hook();
    panic::set_hook(Box::new(|info| {
        let msg = if let Some(s) = info.payload().downcast_ref::<&str>() {
            s.to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "<non-string panic>".to_string()
        };
        let loc = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_else(|| "<unknown>".to_string());
        *LAST_PANIC.lock().unwrap() = Some((msg, loc));
    }));

    // signature -> list of (seed, tick)
    let mut failures: BTreeMap<String, Vec<(u64, u64)>> = BTreeMap::new();
    let mut clean = 0u64;
    let mut agg = Stats::default();

    let total = seeds.len();
    for seed in seeds {
        CURRENT_TICK.store(0, Ordering::Relaxed);
        match panic::catch_unwind(AssertUnwindSafe(|| run_scenario(seed))) {
            Ok(stats) => {
                clean += 1;
                agg.max_term = agg.max_term.max(stats.max_term);
                agg.max_committed = agg.max_committed.max(stats.max_committed);
                agg.truncations += stats.truncations;
                agg.cmds_submitted += stats.cmds_submitted;
            }
            Err(_) => {
                let (msg, loc) = LAST_PANIC
                    .lock()
                    .unwrap()
                    .take()
                    .unwrap_or(("<unknown>".to_string(), "<unknown>".to_string()));
                let tick = CURRENT_TICK.load(Ordering::Relaxed);
                // in single-seed repro mode, show the whole panic message
                if total == 1 {
                    println!("--- seed {} failed at tick {} ({}) ---\n{}\n", seed, tick, loc, msg);
                }
                failures.entry(signature(&msg, &loc)).or_default().push((seed, tick));
            }
        }
    }
    panic::set_hook(old_hook);

    // ---- triage report ----
    println!("\n================ antithesis-style sim report ================");
    println!(
        "seeds run: {} | clean: {} | failing: {}",
        total,
        clean,
        total as u64 - clean
    );
    println!(
        "workload health (clean runs): max term {}, max committed {}, rollbacks {}, commands {}",
        agg.max_term, agg.max_committed, agg.truncations, agg.cmds_submitted
    );
    for (sig, cases) in &failures {
        let (seed, tick) = cases[0];
        println!("\n[{} seed(s)] {}", cases.len(), sig);
        println!(
            "    first repro: seed={} tick={}  (MINIRAFT_SIM_SEED={} cargo test --test antithesis -- --nocapture)",
            seed, tick, seed
        );
    }
    println!("==============================================================\n");

    assert!(
        failures.is_empty(),
        "{} distinct failure signature(s) across {}/{} seeds — see report above",
        failures.len(),
        total as u64 - clean,
        total
    );
}
