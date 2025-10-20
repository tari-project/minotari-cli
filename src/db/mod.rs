use std::{env::current_dir, fs};

use sqlx::sqlite::{SqlitePool, SqlitePoolOptions};

mod accounts;
pub use accounts::{
    AccountBalance, AccountRow, create_account, create_child_account_for_tapplet, get_account_by_name, get_accounts,
    get_balance, get_child_account,
};

mod scanned_tip_blocks;
pub use scanned_tip_blocks::{
    delete_old_scanned_tip_blocks, get_scanned_tip_blocks_by_account, insert_scanned_tip_block,
};

mod outputs;
pub use outputs::{
    DbWalletOutput, fetch_unspent_outputs, get_output_info_by_hash, get_unconfirmed_outputs, insert_output,
    lock_output, mark_output_confirmed, unlock_outputs_for_request, update_output_status,
};

mod pending_transactions;
pub use pending_transactions::{
    PendingTransaction, create_pending_transaction, find_expired_pending_transactions,
    find_pending_transaction_by_idempotency_key, update_pending_transaction_status,
};

mod events;
pub use events::insert_wallet_event;

mod balance_changes;
pub use balance_changes::insert_balance_change;

mod inputs;
pub use inputs::insert_input;

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
