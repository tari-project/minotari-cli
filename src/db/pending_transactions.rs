use chrono::{DateTime, Utc};
use sqlx::{Error as SqlxError, SqliteConnection};
use thiserror::Error;
use uuid::Uuid;

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
    pub status: String,
    pub unsigned_tx_blob: Vec<u8>,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

pub async fn create_pending_transaction(
    conn: &mut SqliteConnection,
    idempotency_key: &str,
    account_id: i64,
    unsigned_tx_json: &str,
    expires_at: DateTime<Utc>,
) -> Result<String, PendingTransactionError> {
    let id = Uuid::new_v4().to_string();
    let status_pending = "PENDING".to_string();

    let res = sqlx::query!(
        r#"
        INSERT INTO pending_transactions (id, idempotency_key, account_id, status, unsigned_tx_json, expires_at)
        VALUES (?, ?, ?, ?, ?, ?)
        "#,
        id,
        idempotency_key,
        account_id,
        status_pending,
        unsigned_tx_json,
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
    sqlx::query_as!(
        ExpiredTransaction,
        r#"
        SELECT id
        FROM pending_transactions
        WHERE status = 'PENDING' AND expires_at < CURRENT_TIMESTAMP
        "#
    )
    .fetch_all(&mut *conn)
    .await
}

pub async fn update_pending_transaction_status(
    conn: &mut SqliteConnection,
    id: &str,
    status: &str,
) -> Result<(), SqlxError> {
    sqlx::query!(
        r#"
        UPDATE pending_transactions
        SET status = ?
        WHERE id = ?
        "#,
        status,
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
) -> Result<Option<String>, SqlxError> {
    let res = sqlx::query!(
        r#"
        SELECT unsigned_tx_json
        FROM pending_transactions
        WHERE idempotency_key = ? AND account_id = ? AND status = 'PENDING'
        "#,
        idempotency_key,
        account_id
    )
    .fetch_optional(&mut *conn)
    .await?;

    Ok(res.map(|r| r.unsigned_tx_json))
}
