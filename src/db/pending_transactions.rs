use crate::models::PendingTransactionStatus;
use chrono::{DateTime, Utc};
use sqlx::{Error as SqlxError, SqliteConnection};
use thiserror::Error;
use uuid::Uuid;

use crate::{api::types::LockFundsResponse, db::outputs::fetch_outputs_by_lock_request_id};
use tari_transaction_components::tari_amount::MicroMinotari;

#[derive(Debug, Error)]
pub enum PendingTransactionError {
    #[error("A pending transaction with the given idempotency key already exists for this account.")]
    DuplicateIdempotencyKey,
    #[error("Database error: {0}")]
    Sqlx(#[from] SqlxError),
}

pub struct PendingTransaction {
    pub id: Uuid,
    pub idempotency_key: String,
    pub account_id: i64,
    pub status: PendingTransactionStatus,
    pub requires_change_output: bool,
    pub total_value: MicroMinotari,
    pub fee_without_change: MicroMinotari,
    pub fee_with_change: MicroMinotari,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

#[allow(clippy::too_many_arguments)]
pub async fn create_pending_transaction(
    conn: &mut SqliteConnection,
    idempotency_key: &str,
    account_id: i64,
    requires_change_output: bool,
    total_value: MicroMinotari,
    fee_without_change: MicroMinotari,
    fee_with_change: MicroMinotari,
    expires_at: DateTime<Utc>,
) -> Result<String, PendingTransactionError> {
    let id = Uuid::new_v4().to_string();
    let status_pending = PendingTransactionStatus::Pending.to_string();
    let total_value = total_value.as_u64() as i64;
    let fee_without_change = fee_without_change.as_u64() as i64;
    let fee_with_change = fee_with_change.as_u64() as i64;

    let res = sqlx::query!(
        r#"
        INSERT INTO pending_transactions (
            id,
            idempotency_key,
            account_id,
            status,
            requires_change_output,
            total_value,
            fee_without_change,
            fee_with_change,
            expires_at
        )
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#,
        id,
        idempotency_key,
        account_id,
        status_pending,
        requires_change_output,
        total_value,
        fee_without_change,
        fee_with_change,
        expires_at
    )
    .execute(&mut *conn)
    .await;

    match res {
        Ok(_) => Ok(id),
        Err(e) => {
            if let SqlxError::Database(db_err) = &e
                && db_err.is_unique_violation()
            {
                return Err(PendingTransactionError::DuplicateIdempotencyKey);
            }
            Err(PendingTransactionError::Sqlx(e))
        },
    }
}

#[derive(sqlx::FromRow, Debug)]
pub struct ExpiredTransaction {
    pub id: String,
}

pub async fn find_expired_pending_transactions(
    conn: &mut SqliteConnection,
) -> Result<Vec<ExpiredTransaction>, SqlxError> {
    let status_pending = PendingTransactionStatus::Pending.to_string();
    sqlx::query_as!(
        ExpiredTransaction,
        r#"
        SELECT id
        FROM pending_transactions
        WHERE status = ? AND expires_at < CURRENT_TIMESTAMP
        "#,
        status_pending
    )
    .fetch_all(&mut *conn)
    .await
}

pub async fn update_pending_transaction_status(
    conn: &mut SqliteConnection,
    id: &str,
    status: PendingTransactionStatus,
) -> Result<(), SqlxError> {
    let status_str = status.to_string();
    sqlx::query!(
        r#"
        UPDATE pending_transactions
        SET status = ?
        WHERE id = ?
        "#,
        status_str,
        id
    )
    .execute(&mut *conn)
    .await?;

    Ok(())
}

pub async fn find_pending_transaction_by_idempotency_key(
    conn: &mut SqliteConnection,
    idempotency_key: &str,
    account_id: i64,
) -> Result<Option<LockFundsResponse>, SqlxError> {
    let status_pending = PendingTransactionStatus::Pending.to_string();
    let res = sqlx::query!(
        r#"
        SELECT
            id,
            requires_change_output,
            total_value,
            fee_without_change,
            fee_with_change
        FROM pending_transactions
        WHERE idempotency_key = ? AND account_id = ? AND status = ?
        "#,
        idempotency_key,
        account_id,
        status_pending
    )
    .fetch_optional(&mut *conn)
    .await?;

    match res {
        Some(row) => {
            let id_str = row.id;
            let utxos = fetch_outputs_by_lock_request_id(conn, &id_str).await?;
            Ok(Some(LockFundsResponse {
                utxos: utxos.into_iter().map(|db_out| db_out.output).collect(),
                requires_change_output: row.requires_change_output,
                total_value: MicroMinotari::from(row.total_value as u64),
                fee_without_change: MicroMinotari::from(row.fee_without_change as u64),
                fee_with_change: MicroMinotari::from(row.fee_with_change as u64),
            }))
        },
        None => Ok(None),
    }
}

pub async fn cancel_pending_transactions_by_ids(
    conn: &mut SqliteConnection,
    ids: &[String],
    status: PendingTransactionStatus,
) -> Result<(), SqlxError> {
    if ids.is_empty() {
        return Ok(());
    }

    let status_str = status.to_string();
    let query_str = format!(
        r#"
        UPDATE pending_transactions
        SET status = ?
        WHERE id IN ({})
        "#,
        ids.iter().map(|_| "?").collect::<Vec<_>>().join(", ")
    );

    let mut query = sqlx::query(&query_str).bind(status_str);
    for id in ids {
        query = query.bind(id);
    }

    query.execute(&mut *conn).await?;

    Ok(())
}
