//! Wallet libraries for Tari
//!
//! This crate provides wallet functionality for the Tari blockchain,
//! including UTXO management, transaction validation, and key management.

// Unit tests may use plain arithmetic; production code is still checked because
// `clippy::arithmetic_side_effects` is linted on the non-test build of this crate.
// (clippy has no `allow-arithmetic-side-effects-in-tests` option.)
#![cfg_attr(test, allow(clippy::arithmetic_side_effects))]

pub mod errors;
mod key_manager_builder;
pub mod scanning;
pub use errors::*;
pub use key_manager_builder::KeyManagerBuilder;
pub use scanning::*;
