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

    // First check if a soft-deleted entry exists and un-delete it
    let existing = sqlx::query!(
        r#"
        SELECT id, deleted_at FROM pending_outputs
        WHERE output_hash = ? AND account_id = ?
        "#,
        output_hash,
        account_id
    )
    .fetch_optional(&mut *conn)
    .await?;

    if let Some(existing_row) = existing {
        if existing_row.deleted_at.is_some() {
            // Un-delete the soft-deleted entry
            sqlx::query!(
                r#"
                UPDATE pending_outputs
                SET deleted_at = NULL, value = ?, wallet_output_json = ?, memo_parsed = ?, memo_hex = ?, status = 'PENDING'
                WHERE id = ?
                "#,
                value,
                output_json,
                memo_parsed,
                memo_hex,
                existing_row.id
            )
            .execute(&mut *conn)
            .await?;
            return Ok((existing_row.id, true)); // Treat un-delete as an insert
        } else {
            // Already exists and is active, just return the ID
            return Ok((existing_row.id, false));
        }
    }

    // Insert new entry
    let insert_result = sqlx::query!(
        r#"
        INSERT INTO pending_outputs (account_id, output_hash, value, wallet_output_json, memo_parsed, memo_hex, status)
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

    Ok((insert_result.last_insert_rowid(), true))
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

/// Get pending output info (id and value) by hash
pub async fn get_pending_output_info_by_hash(
    conn: &mut SqliteConnection,
    output_hash: &[u8],
) -> Result<Option<(i64, u64)>, sqlx::Error> {
    let row = sqlx::query!(
        r#"
        SELECT id, value
        FROM pending_outputs
        WHERE output_hash = ?
        "#,
        output_hash
    )
    .fetch_optional(&mut *conn)
    .await?;

    Ok(row.map(|r| (r.id, r.value as u64)))
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

/// Get all active (non-deleted) pending outputs for an account
pub async fn get_active_pending_outputs(
    conn: &mut SqliteConnection,
    account_id: i64,
) -> Result<Vec<Vec<u8>>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"
        SELECT output_hash
        FROM pending_outputs
        WHERE account_id = ? AND deleted_at IS NULL
        "#,
        account_id
    )
    .fetch_all(&mut *conn)
    .await?;

    Ok(rows.into_iter().map(|r| r.output_hash).collect())
}

/// Soft delete pending outputs that are no longer in the mempool
pub async fn soft_delete_pending_outputs(
    conn: &mut SqliteConnection,
    output_hashes: Vec<Vec<u8>>,
) -> Result<u64, sqlx::Error> {
    if output_hashes.is_empty() {
        return Ok(0);
    }

    let mut affected = 0;
    for hash in output_hashes {
        let result = sqlx::query!(
            r#"
            UPDATE pending_outputs
            SET deleted_at = CURRENT_TIMESTAMP
            WHERE output_hash = ? AND deleted_at IS NULL
            "#,
            hash
        )
        .execute(&mut *conn)
        .await?;
        affected += result.rows_affected();
    }

    Ok(affected)
}
