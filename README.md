# miniraft
### A <1kloc, well-documented Raft consensus algorithm implementation

This crate is a minimal implementation of the Raft consensus protocol with a focus on readability/understandability.

All logic related to the Raft algorithm can be found under `src`. Main files of note are
`src/server.rs` which contains the implementation for a single Raft node and `src/log.rs` which
contains the implementation for an event log which is the basis for the replicated log at the core
of Raft.

This project was created as an exercise in implementing and learning about distributed systems. **Do NOT use this in production.**

[Crate Documentation](https://jzhao.xyz/miniraft)

