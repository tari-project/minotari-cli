//! Transaction-related API endpoint handlers.
//!
//! This module provides HTTP endpoint handlers for querying transaction information
//! in the Minotari wallet REST API.
//!
//! **NOTE:** This file is a mockup provided to demonstrate the required changes,
//! as the original `minotari/src/api/accounts/transactions.rs` was not included
//! in the `REPO FILES`. In a real project, these changes would be integrated
//! into the existing file. Minimal necessary structs are defined for compilation
//! purposes within this file.

use axum::{
    Json,
    extract::{Path, State, Query},
};
use log::debug;
use serde::{Deserialize, Serialize};
use tari_common_types::types::FixedHash;
use tari_transaction_components::tari_amount::MicroMinotari;
use utoipa::{IntoParams, ToSchema};

// --- Mockup/Assumed Structs and Imports ---
// In a real project, these would be imported from `crate::db`, `crate::models`, etc.

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub enum TransactionDirection {
    Inbound,
    Outbound,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub enum TransactionStatus {
    Pending,
    Completed,
    Failed,
    Cancelled,
    New, // Represents newly created/detected transaction
    Rejected,
    Broadcast,
    Mined,
    Confirmed,
    Queued,
    Imported,
    Coinbase,
    OneSided,
    Reorged, // New status to denote a transaction that was in a reorged block
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct DbCompletedTransaction {
    pub id: i64,
    pub direction: TransactionDirection,
    pub status: TransactionStatus,
    #[schema(value_type = u64)]
    pub amount: MicroMinotari,
    #[schema(value_type = u64)]
    pub fee: MicroMinotari,
    pub message: String,
    pub source_pk: Option<String>,
    pub destination_pk: Option<String>,
    pub timestamp: String,
    pub block_height: Option<u64>,
    pub mined_height: Option<u64>,
    pub mined_timestamp: Option<String>,
    pub payref: Option<FixedHash>,
    pub transaction_hex: Option<String>,
    pub kernel_hash: Option<FixedHash>,
    pub original_transaction_id: i64, // The original ID from the `transactions` table
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct DisplayedTransactionOutput {
    pub id: i64,
    pub output_hash: FixedHash,
    #[schema(value_type = u64)]
    pub value: MicroMinotari,
    pub maturity_height: Option<u64>,
    pub status: String, // e.g., "Spent", "Unspent", "Locked", "Invalidated"
    pub is_coinbase: bool,
    pub features: Option<String>, // JSON representation of OutputFeatures
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct DisplayedTransaction {
    pub transaction_id: i64,
    pub direction: TransactionDirection,
    pub status: TransactionStatus,
    #[schema(value_type = u64)]
    pub amount: MicroMinotari,
    #[schema(value_type = u64)]
    pub fee: MicroMinotari,
    pub message: String,
    pub source_pk: Option<String>,
    pub destination_pk: Option<String>,
    pub timestamp: String,
    pub block_height: Option<u64>,
    pub mined_height: Option<u64>,
    pub mined_timestamp: Option<String>,
    pub payref: Option<FixedHash>,
    pub kernel_hash: Option<FixedHash>,
    pub outputs: Vec<DisplayedTransactionOutput>,
}

impl DisplayedTransaction {
    // Mock mapping function - would be actual logic in `models.rs` or `db.rs`
    pub fn from_db_completed(
        tx: DbCompletedTransaction,
        outputs: Vec<DisplayedTransactionOutput>,
    ) -> Self {
        DisplayedTransaction {
            transaction_id: tx.original_transaction_id,
            direction: tx.direction,
            status: tx.status,
            amount: tx.amount,
            fee: tx.fee,
            message: tx.message,
            source_pk: tx.source_pk,
            destination_pk: tx.destination_pk,
            timestamp: tx.timestamp,
            block_height: tx.block_height,
            mined_height: tx.mined_height,
            mined_timestamp: tx.mined_timestamp,
            payref: tx.payref,
            kernel_hash: tx.kernel_hash,
            outputs,
        }
    }
}

// Assumed imports from crate modules
use crate::{
    api::{AppState, error::ApiError},
    db, // Assuming this imports the existing db module.
    wallet_db_extensions, // New import for payref tracking
};

use super::params::{PaginationParams, PayrefParams, WalletParams};

// --- Helper functions to bridge to db module. These would typically be in `db.rs` ---
// Mock implementations for demonstration.

mod mock_db_helpers {
    use super::*;
    use anyhow::anyhow;
    use rusqlite::Connection;

    pub fn get_account_by_name(conn: &Connection, name: &str) -> Result<Option<crate::db::DbAccount>, anyhow::Error> {
        // Mock implementation for demonstration
        // In a real scenario, this would query the `accounts` table.
        // For now, assume "default" exists with ID 1.
        if name == "default" {
            Ok(Some(crate::db::DbAccount {
                id: 1,
                name: "default".to_string(),
                is_default: true,
                key_manager_state: "".to_string(),
                created_at: NaiveDateTime::MIN,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn get_completed_transaction_by_payref(
        _conn: &Connection,
        _account_id: i64,
        _payref: &FixedHash,
    ) -> Result<Option<DbCompletedTransaction>, anyhow::Error> {
        // This is the *original* db function assumed to only look in `completed_transactions`.
        // For this mock, it always returns None, forcing the lookup to the reorg table.
        // In a real implementation, this would query the `completed_transactions` table.
        Ok(None)
    }

    pub fn get_completed_transaction_by_id(
        _conn: &Connection,
        _account_id: i64,
        _transaction_id: i64,
    ) -> Result<Option<DbCompletedTransaction>, anyhow::Error> {
        // Mock implementation: returns a dummy transaction if ID matches (for testing old payref lookup)
        if _transaction_id == 12345 && _account_id == 1 { // Example ID
            Ok(Some(DbCompletedTransaction {
                id: 1,
                direction: TransactionDirection::Inbound,
                status: TransactionStatus::Completed,
                amount: MicroMinotari(1000000),
                fee: MicroMinotari(100),
                message: "Found via old payref".to_string(),
                source_pk: None,
                destination_pk: None,
                timestamp: "2024-01-01T12:00:00Z".to_string(),
                block_height: Some(100),
                mined_height: Some(100),
                mined_timestamp: Some("2024-01-01T12:05:00Z".to_string()),
                payref: Some(FixedHash::zero()), // Assuming a new payref
                transaction_hex: None,
                kernel_hash: None,
                original_transaction_id: 12345,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn get_outputs_for_transaction(
        _conn: &Connection,
        _transaction_id: i64,
    ) -> Result<Vec<DisplayedTransactionOutput>, anyhow::Error> {
        // Mock implementation: returns dummy outputs
        if _transaction_id == 12345 {
            Ok(vec![DisplayedTransactionOutput {
                id: 1,
                output_hash: FixedHash::from_array([1; 32]),
                value: MicroMinotari(1000000),
                maturity_height: Some(200),
                status: "Unspent".to_string(),
                is_coinbase: false,
                features: None,
            }])
        } else {
            Ok(vec![])
        }
    }
}

// Assuming the existing `db` module uses the `mock_db_helpers` functions for this context.
// In a real scenario, this would be a direct call to existing `db` functions.
// For the purpose of this mock, we map them directly.
#[allow(non_snake_case)]
mod db {
    pub use super::mock_db_helpers::{
        get_account_by_name,
        get_completed_transaction_by_id,
        get_completed_transaction_by_payref,
        get_outputs_for_transaction,
    };

    // Dummy DbAccount for compilation. Real one would be in `db.rs` or `models.rs`
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct DbAccount {
        pub id: i64,
        pub name: String,
        pub is_default: bool,
        pub key_manager_state: String,
        pub created_at: chrono::NaiveDateTime,
    }
}
// --- End Mockup/Assumed Structs and Imports ---


/// Helper function to look up a completed transaction by its current or old payref.
///
/// It first attempts to find the transaction using its current `payref` in the `completed_transactions` table.
/// If not found, it checks the `reorg_payrefs` table for a matching old `payref`.
/// If an old `payref` matches, it retrieves the current state of that transaction by its `transaction_id`.
async fn _lookup_completed_transaction_by_any_payref(
    pool: &tari_core::db_sqlite::SqlitePool, // Assumed DB pool type
    account_id: i64,
    payref: FixedHash,
) -> Result<Option<DbCompletedTransaction>, ApiError> {
    let payref_bytes = payref.as_bytes().to_vec();

    tokio::task::spawn_blocking(move || {
        let conn = pool.get().map_err(|e| ApiError::DbError(e.to_string()))?;

        let payref_hash = FixedHash::try_from(payref_bytes)
            .map_err(|e| ApiError::BadRequest(format!("Invalid payref hash format: {}", e)))?;

        // 1. Try to find in the main `completed_transactions` table by current payref
        if let Some(tx) = db::get_completed_transaction_by_payref(&conn, account_id, &payref_hash)
            .map_err(|e| ApiError::DbError(e.to_string()))?
        {
            return Ok(Some(tx));
        }

        // 2. If not found, check the `reorg_payrefs` table for old payrefs
        if let Some(old_tx_id) = wallet_db_extensions::get_transaction_id_by_reorg_payref(&conn, &payref_hash)
            .map_err(|e| ApiError::DbError(e.to_string()))?
        {
            // If an old transaction ID is found, retrieve the *current* state of that transaction
            // from the primary `completed_transactions` table by its `transaction_id`.
            if let Some(tx) = db::get_completed_transaction_by_id(&conn, account_id, old_tx_id)
                .map_err(|e| ApiError::DbError(e.to_string()))?
            {
                return Ok(Some(tx));
            }
        }
        Ok(None)
    })
    .await
    .map_err(|e| ApiError::InternalServerError(format!("Task join error: {}", e)))?
}

/// Helper function to look up a displayed transaction by its current or old payref.
///
/// This uses the `_lookup_completed_transaction_by_any_payref` helper and then
/// maps the `DbCompletedTransaction` to a `DisplayedTransaction` including its outputs.
async fn _lookup_displayed_transaction_by_any_payref(
    pool: &tari_core::db_sqlite::SqlitePool, // Assumed DB pool type
    account_id: i64,
    payref: FixedHash,
) -> Result<Option<DisplayedTransaction>, ApiError> {
    let completed_tx = _lookup_completed_transaction_by_any_payref(pool, account_id, payref).await?;

    if let Some(db_tx) = completed_tx {
        tokio::task::spawn_blocking(move || {
            let conn = pool.get().map_err(|e| ApiError::DbError(e.to_string()))?;
            // Retrieve outputs associated with the found transaction
            let outputs = db::get_outputs_for_transaction(&conn, db_tx.original_transaction_id)
                .map_err(|e| ApiError::DbError(e.to_string()))?;
            // Map the DbCompletedTransaction and its outputs to a DisplayedTransaction
            let displayed_tx = DisplayedTransaction::from_db_completed(db_tx, outputs);
            Ok(Some(displayed_tx))
        })
        .await
        .map_err(|e| ApiError::InternalServerError(format!("Task join error: {}", e)))?
    } else {
        Ok(None)
    }
}

/// Retrieves a completed transaction by its payment reference for a specified account.
///
/// This endpoint searches for the transaction using both its current and any previously
/// used (reorged) payment references.
///
/// # Path Parameters
///
/// - `name`: The unique account name
/// - `payref`: The payment reference hash (as a hex string) to search for
///
/// # Response
///
/// Returns a single [`DbCompletedTransaction`] object if found.
///
/// # Errors
///
/// - [`ApiError::AccountNotFound`]: The specified account does not exist
/// - [`ApiError::TransactionNotFound`]: No transaction found with the given payref
/// - [`ApiError::BadRequest`]: Invalid payref format
/// - [`ApiError::DbError`]: Database connection or query failure
///
/// # Example Request
///
/// 