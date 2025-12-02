use std::{env::current_dir, fs};

use sqlx::sqlite::{SqlitePool, SqlitePoolOptions};

mod accounts;
pub use accounts::{AccountBalance, AccountRow, create_account, get_account_by_name, get_accounts, get_balance};

mod scanned_tip_blocks;
pub use scanned_tip_blocks::{
    delete_scanned_tip_blocks_from_height, get_latest_scanned_tip_block_by_account, get_scanned_tip_blocks_by_account,
    insert_scanned_tip_block, prune_scanned_tip_blocks,
};

mod outputs;
pub use outputs::{
    DbWalletOutput, fetch_outputs_by_lock_request_id, fetch_unspent_outputs,
    get_output_details_for_balance_change_by_id, get_output_info_by_hash, get_unconfirmed_outputs, insert_output,
    lock_output, mark_output_confirmed, soft_delete_outputs_from_height, unlock_outputs_for_request,
    unlock_outputs_for_request as unlock_outputs_for_pending_transaction, update_output_status,
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
    get_completed_transactions_by_status, mark_completed_transaction_as_broadcasted,
    mark_completed_transaction_as_confirmed, mark_completed_transaction_as_mined_unconfirmed,
    mark_completed_transaction_as_rejected, reset_mined_completed_transactions_from_height,
    revert_completed_transaction_to_completed, update_completed_transaction_status,
};

mod events;
pub use events::insert_wallet_event;

mod balance_changes;
pub use balance_changes::{get_all_balance_changes_by_account_id, insert_balance_change};

mod inputs;
pub use inputs::{get_input_details_for_balance_change_by_id, insert_input, soft_delete_inputs_from_height};

pub async fn init_db(db_path: &str) -> Result<SqlitePool, anyhow::Error> {
    let mut path = std::path::Path::new(db_path).to_path_buf();
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
    dbg!(&db_url);
    let pool = SqlitePoolOptions::new().max_connections(5).connect(&db_url).await?;

    // Run migrations
    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}
