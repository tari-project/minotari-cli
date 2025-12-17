//! Database layer for wallet data persistence using SQLite.
//!
//! This module provides the complete database interface for the Minotari wallet,
//! handling all persistent storage including accounts, outputs, transactions,
//! and blockchain scanning state.
//!
//! # Database Schema
//!
//! The database consists of several interconnected tables:
//!
//! - **accounts** - Encrypted wallet keys and account metadata
//! - **outputs** - Detected UTXOs with confirmation status
//! - **inputs** - Spent outputs (transaction inputs)
//! - **balance_changes** - Transaction history with credits and debits
//! - **wallet_events** - Timeline of wallet activity
//! - **scanned_tip_blocks** - Scanning progress and reorg detection
//! - **pending_transactions** - Transactions being constructed
//! - **completed_transactions** - Broadcasted transactions and their status
//! - **displayed_transactions** - User-friendly transaction view
//!
//! # Migrations
//!
//! Database migrations are managed by SQLx and automatically applied on initialization.
//! Migration files are located in the `migrations/` directory.
//!
//! # Usage Example
//!
//! ```no_run
//! use minotari::db::{init_db, get_accounts, get_balance};
//!
//! # async fn example() -> Result<(), anyhow::Error> {
//! // Initialize database and run migrations
//! let pool = init_db("wallet.db").await?;
//!
//! // Query accounts
//! let accounts = get_accounts(&pool, None).await?;
//!
//! // Check balance
//! let balance = get_balance(&pool, "default").await?;
//! println!("Available: {} µT", balance.available);
//! # Ok(())
//! # }
//! ```

use std::{env::current_dir, fs, path::Path};

use sqlx::sqlite::{SqlitePool, SqlitePoolOptions};

mod accounts;
pub use accounts::{
    AccountBalance, AccountRow, AccountTypeRow, ParentAccountRow, create_account, create_child_account_for_tapplet,
    create_child_viewkey_account, get_account_by_id, get_account_by_name, get_accounts, get_balance, get_child_account,
    get_parent_account_by_name,
};

mod scanned_tip_blocks;
pub use scanned_tip_blocks::{
    delete_scanned_tip_blocks_from_height, get_latest_scanned_tip_block_by_account, get_scanned_tip_blocks_by_account,
    insert_scanned_tip_block, prune_scanned_tip_blocks,
};

mod outputs;
pub use outputs::{
    DbWalletOutput, fetch_outputs_by_lock_request_id, fetch_unspent_outputs,
    get_output_details_for_balance_change_by_id, get_output_info_by_hash, get_output_memos_for_account,
    get_unconfirmed_outputs, insert_output, lock_output, mark_output_confirmed, soft_delete_outputs_from_height,
    unlock_outputs_for_request, unlock_outputs_for_request as unlock_outputs_for_pending_transaction,
    update_output_status,
};

mod pending_transactions;
pub use pending_transactions::{
    PendingTransaction, cancel_pending_transactions_by_ids, check_if_transaction_is_expired_by_idempotency_key,
    check_if_transaction_was_already_completed_by_idempotency_key, create_pending_transaction,
    find_expired_pending_transactions, find_pending_transaction_by_idempotency_key,
    find_pending_transaction_locked_funds_by_idempotency_key, update_pending_transaction_status,
};

mod completed_transactions;
pub use completed_transactions::{
    CompletedTransaction, CompletedTransactionStatus, create_completed_transaction, get_completed_transaction_by_id,
    get_completed_transactions_by_status, get_pending_completed_transactions,
    mark_completed_transaction_as_broadcasted, mark_completed_transaction_as_confirmed,
    mark_completed_transaction_as_mined_unconfirmed, mark_completed_transaction_as_rejected,
    reset_mined_completed_transactions_from_height, revert_completed_transaction_to_completed,
    update_completed_transaction_status,
};

mod events;
pub use events::insert_wallet_event;

mod balance_changes;
pub use balance_changes::{get_all_balance_changes_by_account_id, insert_balance_change};

mod inputs;
pub use inputs::{get_input_details_for_balance_change_by_id, insert_input, soft_delete_inputs_from_height};

mod displayed_transactions;
pub use displayed_transactions::{
    find_pending_outbound_by_output_hash, get_displayed_transaction_by_id, get_displayed_transactions_by_account,
    get_displayed_transactions_by_status, get_displayed_transactions_excluding_reorged,
    get_displayed_transactions_from_height, get_displayed_transactions_needing_confirmation_update,
    get_displayed_transactions_paginated, insert_displayed_transaction, mark_displayed_transaction_rejected,
    mark_displayed_transactions_reorganized, mark_displayed_transactions_reorganized_and_return,
    update_displayed_transaction_confirmations, update_displayed_transaction_mined,
    update_displayed_transaction_status,
};

/// Initializes the SQLite database and runs migrations.
///
/// This function:
/// 1. Resolves the database path (handles relative paths)
/// 2. Creates parent directories if they don't exist
/// 3. Creates the database file if it doesn't exist
/// 4. Creates a connection pool with up to 5 connections
/// 5. Runs all pending migrations from `migrations/` directory
///
/// # Parameters
///
/// * `db_path` - Path to the SQLite database file (relative or absolute)
///
/// # Returns
///
/// Returns a `SqlitePool` ready for use, or an error if:
/// - The path is invalid
/// - Directory creation fails
/// - Database file creation fails
/// - Connection fails
/// - Migrations fail
///
/// # Example
///
/// ```no_run
/// use minotari::db::init_db;
///
/// # async fn example() -> Result<(), anyhow::Error> {
/// // Initialize database in current directory
/// let pool = init_db("wallet.db").await?;
///
/// // Or use absolute path
/// let pool = init_db("/path/to/wallet.db").await?;
/// # Ok(())
/// # }
/// ```
pub async fn init_db<P: AsRef<Path>>(db_path: P) -> Result<SqlitePool, anyhow::Error> {
    let mut path = db_path.as_ref().to_path_buf();
    if path.is_relative() {
        path = current_dir()?.to_path_buf().join(path);
    }
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Invalid database file path"))?;
    std::fs::create_dir_all(parent)?;
    if fs::metadata(&path).is_err() {
        fs::File::create(&path)
            .map_err(|e| sqlx::Error::Io(std::io::Error::other(format!("Failed to create database file: {}", e))))?;
    }
    let db_url = format!("sqlite:///{}", path.display().to_string().replace("\\", "/"));
    let pool = SqlitePoolOptions::new().max_connections(5).connect(&db_url).await?;

    // Run migrations
    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}
