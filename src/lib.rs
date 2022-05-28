//! This crate is a minimal implementation of the Raft
//! consensus protocol with a focus on readability/understandability.
//! Do NOT use this in production.
#![warn(missing_docs)]

/// Module for pretty printing state transitions, log updates, etc.
/// No actual Raft-specific logic.
pub mod debug;

/// Module containing implementation for an event log. This is the basis
/// for the replicated log at the core of Raft
pub mod log;

/// Module containing definitions for all of the RPCs that Raft nodes use to
/// communicate with each other.
pub mod rpc;

/// Module containing majority of the logic for handling RPCs, managing state
/// transitions, and the API
pub mod server;
