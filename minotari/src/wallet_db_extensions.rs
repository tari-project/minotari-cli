//! Database extensions for wallet functionality, specifically for tracking reorged payment references.

use anyhow::anyhow;
use chrono::NaiveDateTime;
use rusqlite::{params, Connection, OptionalExtension};
use tari_common_types::types::FixedHash;

/// Represents an entry in the `reorg_payrefs` table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DbReorgPayref {
    pub id: i64,
    pub transaction_id: i64,
    pub output_hash: FixedHash,
    pub payref: FixedHash,
    pub created_at: NaiveDateTime,
}

/// Initializes the `reorg_payrefs` table in the database.
///
/// This function should be called once during application startup, typically when the
/// database connection pool is being set up.
pub fn init_reorg_payrefs_table(conn: &Connection) -> Result<(), anyhow::Error> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS reorg_payrefs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            transaction_id INTEGER NOT NULL,
            output_hash BLOB NOT NULL,
            payref BLOB NOT NULL,
            created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
            FOREIGN KEY (transaction_id) REFERENCES transactions(id) ON DELETE CASCADE,
            UNIQUE (output_hash, payref)
        );",
        [],
    )?;
    Ok(())
}

/// Inserts a new reorged payref entry into the tracking table.
///
/// This function is called when a block reorg affects a transaction that had a payref,
/// making the old payref potentially stale. It stores the old payref linked to the
/// original transaction ID and the specific output hash. `INSERT OR IGNORE` is used
/// to prevent duplicate entries if the same output with the same payref is re-processed.
pub fn insert_reorg_payref_entry(
    conn: &Connection,
    transaction_id: i64,
    output_hash: FixedHash,
    payref: FixedHash,
) -> Result<(), anyhow::Error> {
    conn.execute(
        "INSERT OR IGNORE INTO reorg_payrefs (transaction_id, output_hash, payref) VALUES (?, ?, ?)",
        params![transaction_id, output_hash.as_bytes(), payref.as_bytes()],
    )?;
    Ok(())
}

/// Retrieves a `transaction_id` from the `reorg_payrefs` table given an old payref.
///
/// This is used when a lookup by payref fails in the primary transaction table,
/// suggesting the payref might be an old, reorged one.
pub fn get_transaction_id_by_reorg_payref(
    conn: &Connection,
    payref_hash: &FixedHash,
) -> Result<Option<i64>, anyhow::Error> {
    let mut stmt = conn.prepare(
        "SELECT transaction_id FROM reorg_payrefs WHERE payref = ? LIMIT 1",
    )?;
    let transaction_id = stmt.query_row(
        params![payref_hash.as_bytes()],
        |row| row.get(0),
    ).optional()?;
    Ok(transaction_id)
}
