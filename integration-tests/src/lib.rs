// Cucumber Integration Test Support Library
//
// This library provides test infrastructure for the minotari_cli integration tests,
// including base node process management and test utilities.

// Test-only crate: plain arithmetic is fine here.
#![allow(clippy::arithmetic_side_effects)]

pub mod base_node_process;

pub use base_node_process::{BaseNodeProcess, spawn_base_node};
