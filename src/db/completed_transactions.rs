use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use sqlx::{Error as SqlxError, SqliteConnection};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompletedTransactionStatus {
    Completed,
    Broadcast,
    MinedUnconfirmed,
    MinedConfirmed,
    Rejected,
    Canceled,
}

impl fmt::Display for CompletedTransactionStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CompletedTransactionStatus::Completed => write!(f, "completed"),
            CompletedTransactionStatus::Broadcast => write!(f, "broadcast"),
            CompletedTransactionStatus::MinedUnconfirmed => write!(f, "mined_unconfirmed"),
            CompletedTransactionStatus::MinedConfirmed => write!(f, "mined_confirmed"),
            CompletedTransactionStatus::Rejected => write!(f, "rejected"),
            CompletedTransactionStatus::Canceled => write!(f, "canceled"),
        }
    }
}

impl FromStr for CompletedTransactionStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "completed" => Ok(CompletedTransactionStatus::Completed),
            "broadcast" => Ok(CompletedTransactionStatus::Broadcast),
            "mined_unconfirmed" => Ok(CompletedTransactionStatus::MinedUnconfirmed),
            "mined_confirmed" => Ok(CompletedTransactionStatus::MinedConfirmed),
            "rejected" => Ok(CompletedTransactionStatus::Rejected),
            "canceled" => Ok(CompletedTransactionStatus::Canceled),
            _ => Err(format!("Unknown status: {}", s)),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CompletedTransaction {
    pub id: String,
    pub pending_tx_id: String,
    pub account_id: i64,
    pub status: CompletedTransactionStatus,
    pub last_rejected_reason: Option<String>,
    pub kernel_excess: Vec<u8>,
    pub sent_payref: Option<String>,
    pub mined_height: Option<i64>,
    pub mined_block_hash: Option<Vec<u8>>,
    pub confirmation_height: Option<i64>,
    pub broadcast_attempts: i32,
    pub serialized_transaction: Vec<u8>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub async fn create_completed_transaction(
    conn: &mut SqliteConnection,
    account_id: i64,
    pending_tx_id: &str,
    kernel_excess: &[u8],
    serialized_transaction: &[u8],
    sent_payref: Option<String>,
) -> Result<String, SqlxError> {
    let id = Uuid::new_v4().to_string();
    let status_str = CompletedTransactionStatus::Completed.to_string();

    sqlx::query!(
        r#"
        INSERT INTO completed_transactions (id, account_id, pending_tx_id, status, kernel_excess, serialized_transaction, sent_payref)
        VALUES (?, ?, ?, ?, ?, ?, ?)
        "#,
        id,
        account_id,
        pending_tx_id,
        status_str,
        kernel_excess,
        serialized_transaction,
        sent_payref
    )
    .execute(&mut *conn)
    .await?;

    Ok(id)
}

pub async fn get_completed_transaction_by_id(
    conn: &mut SqliteConnection,
    id: &str,
) -> Result<Option<CompletedTransaction>, SqlxError> {
    let row = sqlx::query!(
        r#"
        SELECT id, account_id, pending_tx_id, status, last_rejected_reason, kernel_excess, 
               sent_payref, mined_height, mined_block_hash, confirmation_height, 
               broadcast_attempts, serialized_transaction, created_at, updated_at
        FROM completed_transactions
        WHERE id = ?
        "#,
        id
    )
    .fetch_optional(&mut *conn)
    .await?;

    match row {
        Some(r) => {
            let status = CompletedTransactionStatus::from_str(&r.status)
                .map_err(|e| SqlxError::Decode(Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, e))))?;

            Ok(Some(CompletedTransaction {
                id: r.id,
                account_id: r.account_id,
                pending_tx_id: r.pending_tx_id,
                status,
                last_rejected_reason: r.last_rejected_reason,
                kernel_excess: r.kernel_excess,
                sent_payref: r.sent_payref,
                mined_height: r.mined_height,
                mined_block_hash: r.mined_block_hash,
                confirmation_height: r.confirmation_height,
                broadcast_attempts: r.broadcast_attempts as i32,
                serialized_transaction: r.serialized_transaction,
                created_at: DateTime::<Utc>::from_naive_utc_and_offset(r.created_at, Utc),
                updated_at: DateTime::<Utc>::from_naive_utc_and_offset(r.updated_at, Utc),
            }))
        },
        None => Ok(None),
    }
}

pub async fn get_completed_transactions_by_status(
    conn: &mut SqliteConnection,
    account_id: i64,
    status: CompletedTransactionStatus,
) -> Result<Vec<CompletedTransaction>, SqlxError> {
    let status_str = status.to_string();

    let rows = sqlx::query!(
        r#"
        SELECT id, account_id, pending_tx_id, status, last_rejected_reason, kernel_excess, 
               sent_payref, mined_height, mined_block_hash, confirmation_height, 
               broadcast_attempts, serialized_transaction, created_at, updated_at
        FROM completed_transactions
        WHERE account_id = ? AND status = ?
        "#,
        account_id,
        status_str
    )
    .fetch_all(&mut *conn)
    .await?;

    let mut result = Vec::with_capacity(rows.len());
    for r in rows {
        let status = CompletedTransactionStatus::from_str(&r.status)
            .map_err(|e| SqlxError::Decode(Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, e))))?;

        result.push(CompletedTransaction {
            id: r.id,
            account_id: r.account_id,
            pending_tx_id: r.pending_tx_id,
            status,
            last_rejected_reason: r.last_rejected_reason,
            kernel_excess: r.kernel_excess,
            sent_payref: r.sent_payref,
            mined_height: r.mined_height,
            mined_block_hash: r.mined_block_hash,
            confirmation_height: r.confirmation_height,
            broadcast_attempts: r.broadcast_attempts as i32,
            serialized_transaction: r.serialized_transaction,
            created_at: DateTime::<Utc>::from_naive_utc_and_offset(r.created_at, Utc),
            updated_at: DateTime::<Utc>::from_naive_utc_and_offset(r.updated_at, Utc),
        });
    }

    Ok(result)
}

pub async fn update_completed_transaction_status(
    conn: &mut SqliteConnection,
    id: &str,
    status: CompletedTransactionStatus,
) -> Result<(), SqlxError> {
    let status_str = status.to_string();
    let now = Utc::now();

    sqlx::query!(
        r#"
        UPDATE completed_transactions
        SET status = ?, updated_at = ?
        WHERE id = ?
        "#,
        status_str,
        now,
        id
    )
    .execute(&mut *conn)
    .await?;

    Ok(())
}

pub async fn mark_completed_transaction_as_broadcasted(
    conn: &mut SqliteConnection,
    id: &str,
    attempts: i32,
) -> Result<(), SqlxError> {
    let status = CompletedTransactionStatus::Broadcast.to_string();
    let now = Utc::now();

    sqlx::query!(
        r#"
        UPDATE completed_transactions
        SET status = ?, broadcast_attempts = ?, updated_at = ?
        WHERE id = ?
        "#,
        status,
        attempts,
        now,
        id
    )
    .execute(&mut *conn)
    .await?;

    Ok(())
}

pub async fn mark_completed_transaction_as_mined_unconfirmed(
    conn: &mut SqliteConnection,
    id: &str,
    block_height: i64,
    block_hash: &[u8],
) -> Result<(), SqlxError> {
    let status = CompletedTransactionStatus::MinedUnconfirmed.to_string();
    let now = Utc::now();

    sqlx::query!(
        r#"
        UPDATE completed_transactions
        SET status = ?, mined_height = ?, mined_block_hash = ?, updated_at = ?
        WHERE id = ?
        "#,
        status,
        block_height,
        block_hash,
        now,
        id
    )
    .execute(&mut *conn)
    .await?;

    Ok(())
}

pub async fn mark_completed_transaction_as_confirmed(
    conn: &mut SqliteConnection,
    id: &str,
    confirmation_height: i64,
) -> Result<(), SqlxError> {
    let status = CompletedTransactionStatus::MinedConfirmed.to_string();
    let now = Utc::now();

    sqlx::query!(
        r#"
        UPDATE completed_transactions
        SET status = ?, confirmation_height = ?, updated_at = ?
        WHERE id = ?
        "#,
        status,
        confirmation_height,
        now,
        id
    )
    .execute(&mut *conn)
    .await?;

    Ok(())
}

pub async fn revert_completed_transaction_to_completed(conn: &mut SqliteConnection, id: &str) -> Result<(), SqlxError> {
    let status = CompletedTransactionStatus::Completed.to_string();
    let now = Utc::now();

    sqlx::query!(
        r#"
        UPDATE completed_transactions
        SET status = ?, mined_height = NULL, mined_block_hash = NULL, 
            confirmation_height = NULL, broadcast_attempts = 0, updated_at = ?
        WHERE id = ?
        "#,
        status,
        now,
        id
    )
    .execute(&mut *conn)
    .await?;

    Ok(())
}

pub async fn reset_mined_completed_transactions_from_height(
    conn: &mut SqliteConnection,
    account_id: i64,
    reorg_height: u64,
) -> Result<u64, SqlxError> {
    let status_completed = CompletedTransactionStatus::Completed.to_string();
    let status_unconfirmed = CompletedTransactionStatus::MinedUnconfirmed.to_string();
    let status_confirmed = CompletedTransactionStatus::MinedConfirmed.to_string();
    let height = reorg_height as i64;
    let now = Utc::now();

    let result = sqlx::query!(
        r#"
        UPDATE completed_transactions
        SET status = ?, mined_height = NULL, mined_block_hash = NULL, 
            confirmation_height = NULL, broadcast_attempts = 0, updated_at = ?
        WHERE account_id = ? AND (status = ? OR status = ?) AND mined_height >= ?
        "#,
        status_completed,
        now,
        account_id,
        status_unconfirmed,
        status_confirmed,
        height
    )
    .execute(&mut *conn)
    .await?;

    Ok(result.rows_affected())
}

pub async fn mark_completed_transaction_as_rejected(
    conn: &mut SqliteConnection,
    id: &str,
    reject_reason: &str,
) -> Result<(), SqlxError> {
    let status_str = CompletedTransactionStatus::Rejected.to_string();
    let now = Utc::now();

    sqlx::query!(
        r#"
        UPDATE completed_transactions
        SET status = ?, last_rejected_reason = ?, updated_at = ?
        WHERE id = ?
        "#,
        status_str,
        reject_reason,
        now,
        id
    )
    .execute(&mut *conn)
    .await?;

    Ok(())
}
