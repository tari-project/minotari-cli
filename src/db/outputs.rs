use crate::models::OutputStatus;
use chrono::{DateTime, Utc, offset};
use serde_json;
use sqlx::SqliteConnection;
use tari_transaction_components::transaction_components::WalletOutput;

#[allow(clippy::too_many_arguments)]
pub async fn insert_output(
    conn: &mut SqliteConnection,
    account_id: i64,
    output_hash: Vec<u8>,
    output: &WalletOutput,
    block_height: u64,
    block_hash: &[u8],
    mined_timestamp: u64,
    memo_parsed: Option<String>,
    memo_hex: Option<String>,
) -> Result<(i64, bool), sqlx::Error> {
    let output_json = serde_json::to_string(&output).map_err(|e| {
        sqlx::Error::Io(std::io::Error::other(format!(
            "Failed to serialize output to JSON: {}",
            e
        )))
    })?;
    let mined_timestamp = DateTime::<Utc>::from_timestamp(mined_timestamp as i64, 0)
        .ok_or_else(|| {
            sqlx::Error::Io(std::io::Error::other(format!(
                "Invalid mined timestamp: {}",
                mined_timestamp
            )))
        })?
        .to_string();
    let block_height = block_height as i64;
    let value = output.value().as_u64() as i64;
    let insert_result = sqlx::query!(
        r#"
       INSERT OR IGNORE INTO outputs (account_id, output_hash, mined_in_block_height, mined_in_block_hash, value, mined_timestamp, wallet_output_json, memo_parsed, memo_hex)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#,
        account_id,
        output_hash,
        block_height,
        block_hash,
        value,
        mined_timestamp,
        output_json,
        memo_parsed,
        memo_hex
           )
    .execute(&mut *conn)
    .await?;

    let rows_affected = insert_result.rows_affected();

    // Now fetch the ID, which is guaranteed to exist
    let output_id = sqlx::query!(
        r#"
        SELECT id FROM outputs WHERE output_hash = ? AND deleted_at IS NULL
        "#,
        output_hash
    )
    .fetch_one(&mut *conn)
    .await?
    .id;

    Ok((output_id, rows_affected > 0))
}

pub async fn get_output_info_by_hash(
    conn: &mut SqliteConnection,
    output_hash: &[u8],
) -> Result<Option<(i64, u64)>, sqlx::Error> {
    let row = sqlx::query!(
        r#"
        SELECT id, value
        FROM outputs
        WHERE output_hash = ? AND deleted_at IS NULL
        "#,
        output_hash
    )
    .fetch_optional(&mut *conn)
    .await?;

    Ok(row.map(|r| (r.id, r.value as u64)))
}

pub async fn get_unconfirmed_outputs(
    conn: &mut SqliteConnection,
    account_id: i64,
    current_height: u64,
    confirmation_blocks: u64,
) -> Result<Vec<(Vec<u8>, u64, Option<String>, Option<String>)>, sqlx::Error> {
    let min_height_to_confirm = current_height.saturating_sub(confirmation_blocks);
    let min_height = min_height_to_confirm as i64;

    let rows = sqlx::query!(
        r#"
        SELECT output_hash, mined_in_block_height, memo_parsed, memo_hex
        FROM outputs o
        WHERE o.account_id = ?
          AND o.mined_in_block_height <= ?
          AND o.confirmed_height IS NULL
          AND o.deleted_at IS NULL
        "#,
        account_id,
        min_height
    )
    .fetch_all(&mut *conn)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| {
            (
                row.output_hash,
                row.mined_in_block_height as u64,
                row.memo_parsed,
                row.memo_hex,
            )
        })
        .collect())
}

pub async fn mark_output_confirmed(
    conn: &mut SqliteConnection,
    output_hash: &[u8],
    confirmed_height: u64,
    confirmed_hash: &[u8],
) -> Result<(), sqlx::Error> {
    let confirmed_height = confirmed_height as i64;
    sqlx::query!(
        r#"
        UPDATE outputs
        SET confirmed_height = ?, confirmed_hash = ?
        WHERE output_hash = ?
        "#,
        confirmed_height,
        confirmed_hash,
        output_hash
    )
    .execute(&mut *conn)
    .await?;

    Ok(())
}

use crate::db::balance_changes::insert_balance_change;
use crate::models::BalanceChange;

pub async fn soft_delete_outputs_from_height(
    conn: &mut SqliteConnection,
    account_id: i64,
    height: u64,
) -> Result<(), sqlx::Error> {
    let height_i64 = height as i64;
    let now = Utc::now().naive_utc().to_string();

    let outputs_to_delete = sqlx::query!(
        r#"
        SELECT id, value, wallet_output_json
        FROM outputs
        WHERE account_id = ? AND mined_in_block_height >= ? AND deleted_at IS NULL
        "#,
        account_id,
        height_i64
    )
    .fetch_all(&mut *conn)
    .await?;

    for output_row in outputs_to_delete {
        let balance_change = BalanceChange {
            account_id,
            caused_by_output_id: Some(output_row.id),
            caused_by_input_id: None,
            description: format!("Reversal: Output found in blockchain scan (reorg at height {})", height),
            balance_credit: 0,
            balance_debit: output_row.value as u64, // Reversing a credit, so debit the value
            effective_date: Utc::now().naive_utc(),
            effective_height: height,
            claimed_recipient_address: None,
            claimed_sender_address: None,
            memo_parsed: None,
            memo_hex: None,
            claimed_fee: None,
            claimed_amount: None,
        };
        insert_balance_change(conn, &balance_change).await?;
    }

    sqlx::query!(
        r#"
        UPDATE outputs
        SET deleted_at = ?, deleted_in_block_height = ?
        WHERE account_id = ? AND mined_in_block_height >= ? AND deleted_at IS NULL
        "#,
        now,
        height_i64,
        account_id,
        height_i64
    )
    .execute(&mut *conn)
    .await?;

    Ok(())
}

pub async fn update_output_status(
    conn: &mut SqliteConnection,
    output_id: i64,
    status: OutputStatus,
) -> Result<(), sqlx::Error> {
    let status = status.to_string();
    sqlx::query!(
        r#"
        UPDATE outputs
        SET status = ?
        WHERE id = ?
        "#,
        status,
        output_id
    )
    .execute(&mut *conn)
    .await?;

    Ok(())
}

pub async fn lock_output(
    conn: &mut SqliteConnection,
    output_id: i64,
    locked_by_request_id: &str,
    locked_at: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    let locked_status = OutputStatus::Locked.to_string();
    let unspent_status = OutputStatus::Unspent.to_string();
    sqlx::query!(
        r#"
        UPDATE outputs
        SET status = ?, locked_by_request_id = ?, locked_at = ?
        WHERE id = ? and status = ?
        "#,
        locked_status,
        locked_by_request_id,
        locked_at,
        output_id,
        unspent_status,
    )
    .execute(&mut *conn)
    .await?;

    Ok(())
}

#[derive(Debug)]
pub struct DbWalletOutput {
    pub id: i64,
    pub output: WalletOutput,
}

pub async fn fetch_unspent_outputs(
    conn: &mut SqliteConnection,
    account_id: i64,
) -> Result<Vec<DbWalletOutput>, sqlx::Error> {
    let rows = sqlx::query!(
        "SELECT id, wallet_output_json FROM outputs WHERE account_id = ? AND status = 'UNSPENT' AND wallet_output_json IS NOT NULL AND deleted_at IS NULL ORDER BY value DESC",
        account_id
    )
    .fetch_all(&mut *conn)
    .await?;

    let mut outputs = Vec::new();
    for row in rows {
        if let Some(json_str) = row.wallet_output_json {
            let output: WalletOutput = serde_json::from_str(&json_str).map_err(|e| {
                sqlx::Error::Io(std::io::Error::other(format!(
                    "Failed to deserialize WalletOutput: {}",
                    e
                )))
            })?;
            outputs.push(DbWalletOutput { id: row.id, output });
        }
    }
    Ok(outputs)
}

pub async fn unlock_outputs_for_request(
    conn: &mut SqliteConnection,
    locked_by_request_id: &str,
) -> Result<(), sqlx::Error> {
    let unspent_status = OutputStatus::Unspent.to_string();
    sqlx::query!(
        r#"
        UPDATE outputs
        SET status = ?, locked_at = NULL, locked_by_request_id = NULL
        WHERE locked_by_request_id = ?
        "#,
        unspent_status,
        locked_by_request_id,
    )
    .execute(&mut *conn)
    .await?;

    Ok(())
}

pub async fn fetch_outputs_by_lock_request_id(
    conn: &mut SqliteConnection,
    locked_by_request_id: &str,
) -> Result<Vec<DbWalletOutput>, sqlx::Error> {
    let rows = sqlx::query!(
        "SELECT id, wallet_output_json FROM outputs WHERE locked_by_request_id = ? and wallet_output_json IS NOT NULL",
        locked_by_request_id
    )
    .fetch_all(&mut *conn)
    .await?;

    let mut outputs = Vec::new();
    for row in rows {
        if let Some(json_str) = row.wallet_output_json {
            let output: WalletOutput = serde_json::from_str(&json_str).map_err(|e| {
                sqlx::Error::Io(std::io::Error::other(format!(
                    "Failed to deserialize WalletOutput: {}",
                    e
                )))
            })?;
            outputs.push(DbWalletOutput { id: row.id, output });
        }
    }
    Ok(outputs)
}

pub async fn get_output_memos_for_account(
    conn: &mut SqliteConnection,
    account_id: i64,
    limit: u32,
    offset: u32,
) -> Result<Vec<(i64, String, String)>, sqlx::Error> {
    let rows = sqlx::query!(
        "SELECT id, memo_parsed, memo_hex FROM outputs WHERE account_id = ? order by id LIMIT ? OFFSET ?",
        account_id,
        limit,
        offset
    )
    .fetch_all(&mut *conn)
    .await?;

    let mut outputs = Vec::new();
    for row in rows {
        let memo_parsed = row.memo_parsed.unwrap_or_default();
        let memo_hex = row.memo_hex.unwrap_or_default();
        outputs.push((row.id, memo_parsed, memo_hex));
    }
    Ok(outputs)
}
