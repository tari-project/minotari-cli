use anyhow::{Context, Result};
use rusqlite::{params, Connection, Row};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PayrefHistoryEntry {
    pub id: i64,
    pub account_id: i64,
    pub tx_id: Vec<u8>,
    pub output_hash: Vec<u8>,
    pub old_payref: String,
    pub new_payref: Option<String>,
    pub reorg_height: i64,
    pub created_at: String,
}

impl PayrefHistoryEntry {
    fn from_row(row: &Row) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            account_id: row.get("account_id")?,
            tx_id: row.get("tx_id")?,
            output_hash: row.get("output_hash")?,
            old_payref: row.get("old_payref")?,
            new_payref: row.get("new_payref")?,
            reorg_height: row.get("reorg_height")?,
            created_at: row.get("created_at")?,
        })
    }
}

pub fn insert_payref_history(
    conn: &Connection,
    account_id: i64,
    tx_id: &[u8],
    output_hash: &[u8],
    old_payref: &str,
    new_payref: Option<&str>,
    reorg_height: i64,
) -> Result<i64> {
    let sql = r#"
        INSERT INTO payref_history (
            account_id, tx_id, output_hash, old_payref, new_payref, reorg_height
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
    "#;
    
    conn.execute(
        sql,
        params![account_id, tx_id, output_hash, old_payref, new_payref, reorg_height],
    )
    .context("Failed to insert payref history entry")?;
    
    Ok(conn.last_insert_rowid())
}

pub fn find_by_payref(
    conn: &Connection,
    account_id: i64,
    payref: &str,
) -> Result<Option<PayrefHistoryEntry>> {
    let sql = r#"
        SELECT id, account_id, tx_id, output_hash, old_payref, new_payref, reorg_height, created_at
        FROM payref_history
        WHERE account_id = ?1 AND (old_payref = ?2 OR new_payref = ?2)
        ORDER BY created_at DESC
        LIMIT 1
    "#;
    
    let mut stmt = conn.prepare(sql)
        .context("Failed to prepare payref history query")?;
    
    let mut rows = stmt.query_map(params![account_id, payref], PayrefHistoryEntry::from_row)
        .context("Failed to execute payref history query")?;
    
    match rows.next() {
        Some(row) => Ok(Some(row.context("Failed to parse payref history row")?)),
        None => Ok(None),
    }
}

pub fn find_by_tx_id(
    conn: &Connection,
    account_id: i64,
    tx_id: &[u8],
) -> Result<Vec<PayrefHistoryEntry>> {
    let sql = r#"
        SELECT id, account_id, tx_id, output_hash, old_payref, new_payref, reorg_height, created_at
        FROM payref_history
        WHERE account_id = ?1 AND tx_id = ?2
        ORDER BY created_at DESC
    "#;
    
    let mut stmt = conn.prepare(sql)
        .context("Failed to prepare payref history by tx_id query")?;
    
    let rows = stmt.query_map(params![account_id, tx_id], PayrefHistoryEntry::from_row)
        .context("Failed to execute payref history by tx_id query")?;
    
    let mut entries = Vec::new();
    for row in rows {
        entries.push(row.context("Failed to parse payref history row")?);
    }
    
    Ok(entries)
}

pub fn get_all_for_account(
    conn: &Connection,
    account_id: i64,
) -> Result<Vec<PayrefHistoryEntry>> {
    let sql = r#"
        SELECT id, account_id, tx_id, output_hash, old_payref, new_payref, reorg_height, created_at
        FROM payref_history
        WHERE account_id = ?1
        ORDER BY created_at DESC
    "#;
    
    let mut stmt = conn.prepare(sql)
        .context("Failed to prepare payref history list query")?;
    
    let rows = stmt.query_map(params![account_id], PayrefHistoryEntry::from_row)
        .context("Failed to execute payref history list query")?;
    
    let mut entries = Vec::new();
    for row in rows {
        entries.push(row.context("Failed to parse payref history row")?);
    }
    
    Ok(entries)
}
