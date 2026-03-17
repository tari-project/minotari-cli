//! Wallet libraries for Tari
//!
//! This crate provides wallet functionality for the Tari blockchain,
//! including UTXO management, transaction validation, and key management.

pub mod errors;
mod key_manager_builder;
pub mod scanning;
pub use errors::*;
pub use key_manager_builder::KeyManagerBuilder;
pub use scanning::*;
