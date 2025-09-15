use sqlx::sqlite::{SqlitePool, SqlitePoolOptions};
use std::fs;
use std::path::Path;

mod accounts;
pub use accounts::AccountRow;
pub use accounts::create_account;
pub use accounts::get_account_by_name;

pub async fn init_db(db_path: &Path) -> Result<SqlitePool, sqlx::Error> {
    if fs::metadata(db_path).is_err() {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                sqlx::Error::Io(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("Failed to create parent directories: {}", e),
                ))
            })?;
        }
        fs::File::create(db_path).map_err(|e| {
            sqlx::Error::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to create database file: {}", e),
            ))
        })?;
    }
    let db_url = format!(
        "sqlite:///{}",
        db_path.display().to_string().replace("\\", "/")
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
