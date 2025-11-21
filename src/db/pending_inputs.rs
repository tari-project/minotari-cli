use sqlx::SqliteConnection;

/// Insert or update a pending input in the database
/// Returns (pending_input_id, was_inserted)
pub async fn upsert_pending_input(
    conn: &mut SqliteConnection,
    account_id: i64,
    output_id: Option<i64>,
    pending_output_id: Option<i64>,
) -> Result<(i64, bool), sqlx::Error> {
    // Try to insert first
    let insert_result = sqlx::query!(
        r#"
        INSERT OR IGNORE INTO pending_inputs (account_id, output_id, pending_output_id, status)
        VALUES (?, ?, ?, 'PENDING')
        "#,
        account_id,
        output_id,
        pending_output_id
    )
    .execute(&mut *conn)
    .await?;

    let rows_affected = insert_result.rows_affected();

    // Fetch the ID - we need to find by the unique combination
    let pending_input_id = if let Some(oid) = output_id {
        sqlx::query!(
            r#"
            SELECT id FROM pending_inputs
            WHERE account_id = ? AND output_id = ?
            ORDER BY created_at DESC
            LIMIT 1
            "#,
            account_id,
            oid
        )
        .fetch_one(&mut *conn)
        .await?
        .id
    } else if let Some(poid) = pending_output_id {
        sqlx::query!(
            r#"
            SELECT id FROM pending_inputs
            WHERE account_id = ? AND pending_output_id = ?
            ORDER BY created_at DESC
            LIMIT 1
            "#,
            account_id,
            poid
        )
        .fetch_one(&mut *conn)
        .await?
        .id
    } else {
        return Err(sqlx::Error::Io(std::io::Error::other(
            "Either output_id or pending_output_id must be provided",
        )));
    };

    Ok((pending_input_id, rows_affected > 0))
}

/// Get pending input by output_id
pub async fn get_pending_input_by_output_id(
    conn: &mut SqliteConnection,
    output_id: i64,
) -> Result<Option<i64>, sqlx::Error> {
    let row = sqlx::query!(
        r#"
        SELECT id
        FROM pending_inputs
        WHERE output_id = ?
        "#,
        output_id
    )
    .fetch_optional(&mut *conn)
    .await?;

    Ok(row.map(|r| r.id))
}

/// Delete pending input when it's confirmed on-chain
pub async fn delete_pending_input_by_output_id(
    conn: &mut SqliteConnection,
    output_id: i64,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query!(
        r#"
        DELETE FROM pending_inputs WHERE output_id = ?
        "#,
        output_id
    )
    .execute(&mut *conn)
    .await?;

    Ok(result.rows_affected())
}

/// Clean up expired or old pending inputs
pub async fn cleanup_pending_inputs(conn: &mut SqliteConnection, expiry_hours: i64) -> Result<u64, sqlx::Error> {
    let result = sqlx::query!(
        r#"
        DELETE FROM pending_inputs
        WHERE datetime(created_at) < datetime('now', '-' || ? || ' hours')
        "#,
        expiry_hours
    )
    .execute(&mut *conn)
    .await?;

    Ok(result.rows_affected())
}

/// Get all active (non-deleted) pending inputs for an account by output_id
pub async fn get_active_pending_inputs_by_output(
    conn: &mut SqliteConnection,
    account_id: i64,
) -> Result<Vec<i64>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"
        SELECT output_id
        FROM pending_inputs
        WHERE account_id = ? AND deleted_at IS NULL AND output_id IS NOT NULL
        "#,
        account_id
    )
    .fetch_all(&mut *conn)
    .await?;

    Ok(rows.into_iter().filter_map(|r| r.output_id).collect())
}

/// Soft delete pending inputs that are no longer in the mempool
pub async fn soft_delete_pending_inputs_by_output(
    conn: &mut SqliteConnection,
    output_ids: Vec<i64>,
) -> Result<u64, sqlx::Error> {
    if output_ids.is_empty() {
        return Ok(0);
    }

    let mut affected = 0;
    for oid in output_ids {
        let result = sqlx::query!(
            r#"
            UPDATE pending_inputs
            SET deleted_at = CURRENT_TIMESTAMP
            WHERE output_id = ? AND deleted_at IS NULL
            "#,
            oid
        )
        .execute(&mut *conn)
        .await?;
        affected += result.rows_affected();
    }

    Ok(affected)
}
