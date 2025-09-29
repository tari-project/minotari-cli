use sqlx::sqlite::{SqlitePool, SqlitePoolOptions};
use std::env::current_dir;
use std::fs;
use std::path::Path;

mod accounts;
pub use accounts::AccountRow;
pub use accounts::create_account;
pub use accounts::get_account_by_name;
pub use accounts::get_accounts;

mod scanned_tip_blocks;
pub use scanned_tip_blocks::delete_old_scanned_tip_blocks;
pub use scanned_tip_blocks::get_scanned_tip_blocks_by_account;
pub use scanned_tip_blocks::insert_scanned_tip_block;

mod outputs;
pub use outputs::get_output_info_by_hash;
pub use outputs::get_unconfirmed_outputs;
pub use outputs::insert_output;
pub use outputs::mark_output_confirmed;

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
        fs::File::create(&path).map_err(|e| {
            sqlx::Error::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to create database file: {}", e),
            ))
        })?;
    }
    let db_url = format!(
        "sqlite:///{}",
        path.display().to_string().replace("\\", "/")
    );
    dbg!(&db_url);
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await?;

    // Run migrations
    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}
