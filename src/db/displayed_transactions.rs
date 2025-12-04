use chrono::Utc;
use sqlx::{Error as SqlxError, SqliteConnection};

use crate::models::Id;
use crate::transactions::{DisplayedTransaction, TransactionDisplayStatus};

pub async fn insert_displayed_transaction(
    conn: &mut SqliteConnection,
    transaction: &DisplayedTransaction,
) -> Result<(), SqlxError> {
    let direction = format!("{:?}", transaction.direction).to_lowercase();
    let source = format!("{:?}", transaction.source).to_lowercase();
    let status = format!("{:?}", transaction.status).to_lowercase();
    let timestamp = transaction.blockchain.timestamp.format("%Y-%m-%d %H:%M:%S").to_string();
    let transaction_json = serde_json::to_string(transaction).map_err(|e| SqlxError::Encode(Box::new(e)))?;
    let now = Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let amount = transaction.amount as i64;
    let block_height = transaction.blockchain.block_height as i64;

    sqlx::query!(
        r#"
        INSERT INTO displayed_transactions (id, account_id, direction, source, status, amount, block_height, timestamp, transaction_json, created_at, updated_at)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(id) DO UPDATE SET
            status = excluded.status,
            transaction_json = excluded.transaction_json,
            updated_at = excluded.updated_at
        "#,
        transaction.id,
        transaction.details.account_id,
        direction,
        source,
        status,
        amount,
        block_height,
        timestamp,
        transaction_json,
        now,
        now
    )
    .execute(&mut *conn)
    .await?;

    Ok(())
}

pub async fn get_displayed_transactions_by_account(
    conn: &mut SqliteConnection,
    account_id: Id,
) -> Result<Vec<DisplayedTransaction>, SqlxError> {
    let rows = sqlx::query!(
        r#"
        SELECT transaction_json
        FROM displayed_transactions
        WHERE account_id = ?
        ORDER BY block_height DESC, timestamp DESC
        "#,
        account_id
    )
    .fetch_all(&mut *conn)
    .await?;

    let transactions: Vec<DisplayedTransaction> = rows
        .into_iter()
        .filter_map(|row| serde_json::from_str(&row.transaction_json).ok())
        .collect();

    Ok(transactions)
}

pub async fn get_displayed_transactions_by_status(
    conn: &mut SqliteConnection,
    account_id: Id,
    status: TransactionDisplayStatus,
) -> Result<Vec<DisplayedTransaction>, SqlxError> {
    let status_str = format!("{:?}", status).to_lowercase();

    let rows = sqlx::query!(
        r#"
        SELECT transaction_json
        FROM displayed_transactions
        WHERE account_id = ? AND status = ?
        ORDER BY block_height DESC, timestamp DESC
        "#,
        account_id,
        status_str
    )
    .fetch_all(&mut *conn)
    .await?;

    let transactions: Vec<DisplayedTransaction> = rows
        .into_iter()
        .filter_map(|row| serde_json::from_str(&row.transaction_json).ok())
        .collect();

    Ok(transactions)
}

pub async fn get_displayed_transactions_from_height(
    conn: &mut SqliteConnection,
    account_id: Id,
    from_height: u64,
) -> Result<Vec<DisplayedTransaction>, SqlxError> {
    let height = from_height as i64;

    let rows = sqlx::query!(
        r#"
        SELECT transaction_json
        FROM displayed_transactions
        WHERE account_id = ? AND block_height >= ?
        ORDER BY block_height DESC, timestamp DESC
        "#,
        account_id,
        height
    )
    .fetch_all(&mut *conn)
    .await?;

    let transactions: Vec<DisplayedTransaction> = rows
        .into_iter()
        .filter_map(|row| serde_json::from_str(&row.transaction_json).ok())
        .collect();

    Ok(transactions)
}

pub async fn update_displayed_transaction_status(
    conn: &mut SqliteConnection,
    id: &str,
    new_status: TransactionDisplayStatus,
    updated_transaction: &DisplayedTransaction,
) -> Result<bool, SqlxError> {
    let status_str = format!("{:?}", new_status).to_lowercase();
    let transaction_json = serde_json::to_string(updated_transaction).map_err(|e| SqlxError::Encode(Box::new(e)))?;
    let now = Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();

    let result = sqlx::query!(
        r#"
        UPDATE displayed_transactions
        SET status = ?, transaction_json = ?, updated_at = ?
        WHERE id = ?
        "#,
        status_str,
        transaction_json,
        now,
        id
    )
    .execute(&mut *conn)
    .await?;

    Ok(result.rows_affected() > 0)
}

pub async fn mark_displayed_transactions_reorganized(
    conn: &mut SqliteConnection,
    account_id: Id,
    from_height: u64,
) -> Result<u64, SqlxError> {
    let height = from_height as i64;
    let status_str = format!("{:?}", TransactionDisplayStatus::Reorganized).to_lowercase();
    let now = Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();

    // First get all affected transactions
    let rows = sqlx::query!(
        r#"
        SELECT id, transaction_json
        FROM displayed_transactions
        WHERE account_id = ? AND block_height >= ?
        "#,
        account_id,
        height
    )
    .fetch_all(&mut *conn)
    .await?;

    let mut updated_count = 0u64;

    for row in rows {
        if let Ok(mut tx) = serde_json::from_str::<DisplayedTransaction>(&row.transaction_json) {
            tx.status = TransactionDisplayStatus::Reorganized;
            let updated_json = serde_json::to_string(&tx).map_err(|e| SqlxError::Encode(Box::new(e)))?;

            sqlx::query!(
                r#"
                UPDATE displayed_transactions
                SET status = ?, transaction_json = ?, updated_at = ?
                WHERE id = ?
                "#,
                status_str,
                updated_json,
                now,
                row.id
            )
            .execute(&mut *conn)
            .await?;

            updated_count += 1;
        }
    }

    Ok(updated_count)
}

pub async fn get_displayed_transaction_by_id(
    conn: &mut SqliteConnection,
    id: &str,
) -> Result<Option<DisplayedTransaction>, SqlxError> {
    let row = sqlx::query!(
        r#"
        SELECT transaction_json
        FROM displayed_transactions
        WHERE id = ?
        "#,
        id
    )
    .fetch_optional(&mut *conn)
    .await?;

    Ok(row.and_then(|r| serde_json::from_str(&r.transaction_json).ok()))
}

/// Find an existing pending outbound transaction that matches the given output hash.
/// Used by BlockProcessor to detect if a scanned transaction already has a pending record.
pub async fn find_pending_outbound_by_output_hash(
    conn: &mut SqliteConnection,
    account_id: Id,
    output_hash: &str,
) -> Result<Option<DisplayedTransaction>, SqlxError> {
    let pending_status = format!("{:?}", TransactionDisplayStatus::Pending).to_lowercase();
    let outgoing_direction = "outgoing";

    let rows = sqlx::query!(
        r#"
        SELECT transaction_json
        FROM displayed_transactions
        WHERE account_id = ? AND status = ? AND direction = ?
        "#,
        account_id,
        pending_status,
        outgoing_direction
    )
    .fetch_all(&mut *conn)
    .await?;

    for row in rows {
        if let Ok(tx) = serde_json::from_str::<DisplayedTransaction>(&row.transaction_json) {
            // Check if any of the sent_output_hashes match
            if tx.details.sent_output_hashes.contains(&output_hash.to_string()) {
                return Ok(Some(tx));
            }
            // Also check inputs (spent UTXOs)
            if tx.details.inputs.iter().any(|input| input.output_hash == output_hash) {
                return Ok(Some(tx));
            }
        }
    }

    Ok(None)
}

/// Update an existing displayed transaction with blockchain info when it's mined.
pub async fn update_displayed_transaction_mined(
    conn: &mut SqliteConnection,
    tx: &DisplayedTransaction,
) -> Result<bool, SqlxError> {
    let status_str = format!("{:?}", tx.status).to_lowercase();
    let transaction_json = serde_json::to_string(tx).map_err(|e| SqlxError::Encode(Box::new(e)))?;
    let now = Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let block_height = tx.blockchain.block_height as i64;

    let result = sqlx::query!(
        r#"
        UPDATE displayed_transactions
        SET status = ?, block_height = ?, transaction_json = ?, updated_at = ?
        WHERE id = ?
        "#,
        status_str,
        block_height,
        transaction_json,
        now,
        tx.id
    )
    .execute(&mut *conn)
    .await?;

    Ok(result.rows_affected() > 0)
}

pub async fn get_displayed_transactions_paginated(
    conn: &mut SqliteConnection,
    account_id: Id,
    limit: i64,
    offset: i64,
) -> Result<Vec<DisplayedTransaction>, SqlxError> {
    let rows = sqlx::query!(
        r#"
        SELECT transaction_json
        FROM displayed_transactions
        WHERE account_id = ?
        ORDER BY block_height DESC, timestamp DESC
        LIMIT ? OFFSET ?
        "#,
        account_id,
        limit,
        offset
    )
    .fetch_all(&mut *conn)
    .await?;

    let transactions: Vec<DisplayedTransaction> = rows
        .into_iter()
        .filter_map(|row| serde_json::from_str(&row.transaction_json).ok())
        .collect();

    Ok(transactions)
}

/// Returns transactions where current_tip_height - block_height < required_confirmations.
pub async fn get_displayed_transactions_needing_confirmation_update(
    conn: &mut SqliteConnection,
    account_id: Id,
    current_tip_height: u64,
    required_confirmations: u64,
) -> Result<Vec<DisplayedTransaction>, SqlxError> {
    let min_height = current_tip_height.saturating_sub(required_confirmations) as i64;
    let pending_status = format!("{:?}", TransactionDisplayStatus::Pending).to_lowercase();
    let unconfirmed_status = format!("{:?}", TransactionDisplayStatus::Unconfirmed).to_lowercase();

    let rows = sqlx::query!(
        r#"
        SELECT transaction_json
        FROM displayed_transactions
        WHERE account_id = ? 
          AND block_height > ?
          AND status IN (?, ?)
        ORDER BY block_height DESC
        "#,
        account_id,
        min_height,
        pending_status,
        unconfirmed_status
    )
    .fetch_all(&mut *conn)
    .await?;

    let transactions: Vec<DisplayedTransaction> = rows
        .into_iter()
        .filter_map(|row| serde_json::from_str(&row.transaction_json).ok())
        .collect();

    Ok(transactions)
}

pub async fn update_displayed_transaction_confirmations(
    conn: &mut SqliteConnection,
    transaction: &DisplayedTransaction,
) -> Result<bool, SqlxError> {
    let status_str = format!("{:?}", transaction.status).to_lowercase();
    let transaction_json = serde_json::to_string(transaction).map_err(|e| SqlxError::Encode(Box::new(e)))?;
    let now = Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();

    let result = sqlx::query!(
        r#"
        UPDATE displayed_transactions
        SET status = ?, transaction_json = ?, updated_at = ?
        WHERE id = ?
        "#,
        status_str,
        transaction_json,
        now,
        transaction.id
    )
    .execute(&mut *conn)
    .await?;

    Ok(result.rows_affected() > 0)
}

pub async fn mark_displayed_transaction_rejected(
    conn: &mut SqliteConnection,
    tx_id: &str,
) -> Result<Option<DisplayedTransaction>, SqlxError> {
    let status_str = format!("{:?}", TransactionDisplayStatus::Rejected).to_lowercase();
    let now = Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();

    // First get the transaction
    let row = sqlx::query!(
        r#"
        SELECT transaction_json
        FROM displayed_transactions
        WHERE id = ?
        "#,
        tx_id
    )
    .fetch_optional(&mut *conn)
    .await?;

    let Some(row) = row else {
        return Ok(None);
    };

    let mut tx: DisplayedTransaction =
        serde_json::from_str(&row.transaction_json).map_err(|e| SqlxError::Decode(Box::new(e)))?;

    tx.status = TransactionDisplayStatus::Rejected;
    let updated_json = serde_json::to_string(&tx).map_err(|e| SqlxError::Encode(Box::new(e)))?;

    sqlx::query!(
        r#"
        UPDATE displayed_transactions
        SET status = ?, transaction_json = ?, updated_at = ?
        WHERE id = ?
        "#,
        status_str,
        updated_json,
        now,
        tx_id
    )
    .execute(&mut *conn)
    .await?;

    Ok(Some(tx))
}

pub async fn get_displayed_transactions_excluding_reorged(
    conn: &mut SqliteConnection,
    account_id: Id,
) -> Result<Vec<DisplayedTransaction>, SqlxError> {
    let reorged_status = format!("{:?}", TransactionDisplayStatus::Reorganized).to_lowercase();

    let rows = sqlx::query!(
        r#"
        SELECT transaction_json
        FROM displayed_transactions
        WHERE account_id = ? AND status != ?
        ORDER BY block_height DESC, timestamp DESC
        "#,
        account_id,
        reorged_status
    )
    .fetch_all(&mut *conn)
    .await?;

    let transactions: Vec<DisplayedTransaction> = rows
        .into_iter()
        .filter_map(|row| serde_json::from_str(&row.transaction_json).ok())
        .collect();

    Ok(transactions)
}

pub async fn mark_displayed_transactions_reorganized_and_return(
    conn: &mut SqliteConnection,
    account_id: Id,
    from_height: u64,
) -> Result<Vec<DisplayedTransaction>, SqlxError> {
    let height = from_height as i64;
    let status_str = format!("{:?}", TransactionDisplayStatus::Reorganized).to_lowercase();
    let now = Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();

    // First get all affected transactions
    let rows = sqlx::query!(
        r#"
        SELECT id, transaction_json
        FROM displayed_transactions
        WHERE account_id = ? AND block_height >= ? AND status != ?
        "#,
        account_id,
        height,
        status_str
    )
    .fetch_all(&mut *conn)
    .await?;

    let mut updated_transactions = Vec::new();

    for row in rows {
        if let Ok(mut tx) = serde_json::from_str::<DisplayedTransaction>(&row.transaction_json) {
            tx.status = TransactionDisplayStatus::Reorganized;
            let updated_json = serde_json::to_string(&tx).map_err(|e| SqlxError::Encode(Box::new(e)))?;

            sqlx::query!(
                r#"
                UPDATE displayed_transactions
                SET status = ?, transaction_json = ?, updated_at = ?
                WHERE id = ?
                "#,
                status_str,
                updated_json,
                now,
                row.id
            )
            .execute(&mut *conn)
            .await?;

            updated_transactions.push(tx);
        }
    }

    Ok(updated_transactions)
}
