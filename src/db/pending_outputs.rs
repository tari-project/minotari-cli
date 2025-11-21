use serde_json;
use sqlx::SqliteConnection;
use tari_transaction_components::transaction_components::TransactionOutput;

/// Insert or update a pending output in the database
/// Returns (pending_output_id, was_inserted)
/// Note: value must be provided as TransactionOutput doesn't expose it directly (it's in the commitment)
pub async fn upsert_pending_output(
    conn: &mut SqliteConnection,
    account_id: i64,
    output_hash: Vec<u8>,
    output: &TransactionOutput,
    value: u64,
    memo_parsed: String,
    memo_hex: Option<String>,
) -> Result<(i64, bool), sqlx::Error> {
    let output_json = serde_json::to_string(&output).map_err(|e| {
        sqlx::Error::Io(std::io::Error::other(format!(
            "Failed to serialize output to JSON: {}",
            e
        )))
    })?;

    let value = value as i64;

    // Try to insert first
    let insert_result = sqlx::query!(
        r#"
        INSERT OR IGNORE INTO pending_outputs (account_id, output_hash, value, wallet_output_json, memo_parsed, memo_hex, status)
        VALUES (?, ?, ?, ?, ?, ?, 'PENDING')
        "#,
        account_id,
        output_hash,
        value,
        output_json,
        memo_parsed,
        memo_hex
    )
    .execute(&mut *conn)
    .await?;

    let rows_affected = insert_result.rows_affected();

    // Fetch the ID
    let pending_output_id = sqlx::query!(
        r#"
        SELECT id FROM pending_outputs WHERE output_hash = ? AND account_id = ?
        "#,
        output_hash,
        account_id
    )
    .fetch_one(&mut *conn)
    .await?
    .id;

    Ok((pending_output_id, rows_affected > 0))
}

/// Get pending output by hash
pub async fn get_pending_output_by_hash(
    conn: &mut SqliteConnection,
    output_hash: &[u8],
) -> Result<Option<i64>, sqlx::Error> {
    let row = sqlx::query!(
        r#"
        SELECT id
        FROM pending_outputs
        WHERE output_hash = ?
        "#,
        output_hash
    )
    .fetch_optional(&mut *conn)
    .await?;

    Ok(row.map(|r| r.id))
}

/// Delete pending output when it's confirmed on-chain
pub async fn delete_pending_output_by_hash(
    conn: &mut SqliteConnection,
    output_hash: &[u8],
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query!(
        r#"
        DELETE FROM pending_outputs WHERE output_hash = ?
        "#,
        output_hash
    )
    .execute(&mut *conn)
    .await?;

    Ok(result.rows_affected())
}

/// Clean up expired or old pending outputs
pub async fn cleanup_pending_outputs(conn: &mut SqliteConnection, expiry_hours: i64) -> Result<u64, sqlx::Error> {
    let result = sqlx::query!(
        r#"
        DELETE FROM pending_outputs
        WHERE datetime(created_at) < datetime('now', '-' || ? || ' hours')
        "#,
        expiry_hours
    )
    .execute(&mut *conn)
    .await?;

    Ok(result.rows_affected())
}
