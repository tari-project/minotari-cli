use sqlx::SqlitePool;

use crate::db;
use crate::models::Id;
use crate::scan::DisplayedTransactionsEvent;
use crate::transactions::{DisplayedTransaction, DisplayedTransactionProcessor, ProcessorError};

#[derive(Debug, thiserror::Error)]
pub enum TransactionHistoryError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Processing error: {0}")]
    ProcessingError(#[from] ProcessorError),
}

/// Service for managing transaction history.
pub struct TransactionHistoryService {
    db_pool: SqlitePool,
}

impl TransactionHistoryService {
    pub fn new(db_pool: SqlitePool) -> Self {
        Self { db_pool }
    }

    /// Load all transaction history for an account, sorted by timestamp (newest first).
    pub async fn load_all_transactions(
        &self,
        account_id: Id,
    ) -> Result<Vec<DisplayedTransaction>, TransactionHistoryError> {
        let mut conn = self.db_pool.acquire().await?;

        let tip_height = db::get_latest_scanned_tip_block_by_account(&mut conn, account_id)
            .await?
            .map(|block| block.height)
            .unwrap_or(0);

        let processor = DisplayedTransactionProcessor::new(tip_height);
        let transactions = processor
            .process_all_stored_with_conn(account_id, &mut conn, &self.db_pool)
            .await?;

        Ok(transactions)
    }

    /// Load transactions and emit them via the provided callback.
    pub async fn load_and_emit<F>(&self, account_id: Id, emit_fn: F) -> Result<usize, TransactionHistoryError>
    where
        F: FnOnce(DisplayedTransactionsEvent),
    {
        let transactions = self.load_all_transactions(account_id).await?;
        let count = transactions.len();

        if !transactions.is_empty() {
            emit_fn(DisplayedTransactionsEvent {
                account_id,
                transactions,
                block_height: None,
                is_initial_sync: true,
            });
        }

        Ok(count)
    }

    /// Load transactions and emit them in chunks for large histories.
    pub async fn emit_in_chunks<F>(
        &self,
        account_id: Id,
        chunk_size: usize,
        mut emit_fn: F,
    ) -> Result<usize, TransactionHistoryError>
    where
        F: FnMut(DisplayedTransactionsEvent),
    {
        let all_transactions = self.load_all_transactions(account_id).await?;
        let total = all_transactions.len();

        for chunk in all_transactions.chunks(chunk_size) {
            emit_fn(DisplayedTransactionsEvent {
                account_id,
                transactions: chunk.to_vec(),
                block_height: None,
                is_initial_sync: true,
            });
        }

        Ok(total)
    }

    // TODO: Add method like "load_in_batches" that uses pagination for loading transaction from database and process in batches
}
