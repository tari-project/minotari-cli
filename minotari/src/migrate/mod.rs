// Copyright 2026 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

//! Console wallet migration module.
//!
//! This module implements the maintainer's requested "outputs-driven" migration
//! approach:
//!
//! > "We need to construct the timeline, using **mostly the outputs**, with help
//! > of the transactions."
//!
//! # Module structure
//!
//! - `console_db`: Read-only access to legacy console wallet SQLite DB
//! - `output_converter`: Converts legacy `ConsoleOutputRow` → new `WalletOutput`
//! - `tx_converter`: Enriches displayed transaction with legacy tx metadata
//! - `output_driven_migrator`: **The winning implementation** — walks outputs
//!   in `mined_height` order, writing to new wallet

mod console_db;
mod output_converter;
mod tx_converter;
mod output_driven_migrator;

pub use console_db::{
    ConsoleWalletReader, ConsoleOutputRow, ConsoleCompletedTx,
    OUTPUT_STATUS_UNSPENT, OUTPUT_STATUS_SPENT,
    STATUS_COMPLETED,
};

pub use output_converter::convert_output;
pub use tx_converter::enrich_with_transaction_metadata;
pub use output_driven_migrator::{
    OutputDrivenMigrationOptions,
    MigrationResult,
    run_output_driven_migration,
};
