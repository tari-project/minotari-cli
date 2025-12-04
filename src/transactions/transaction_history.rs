use sqlx::SqlitePool;

use crate::db;
use crate::models::Id;
use crate::scan::DisplayedTransactionsEvent;
use crate::transactions::{
    DisplayedTransaction, DisplayedTransactionProcessor, ProcessorError, TransactionDisplayStatus,
};

#[derive(Debug, thiserror::Error)]
pub enum TransactionHistoryError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Processing error: {0}")]
    ProcessingError(#[from] ProcessorError),
}

pub struct TransactionHistoryService {
    db_pool: SqlitePool,
}

impl TransactionHistoryService {
    pub fn new(db_pool: SqlitePool) -> Self {
        Self { db_pool }
    }

    pub async fn load_all_transactions(
        &self,
        account_id: Id,
    ) -> Result<Vec<DisplayedTransaction>, TransactionHistoryError> {
        let mut conn = self.db_pool.acquire().await?;
        let transactions = db::get_displayed_transactions_by_account(&mut conn, account_id).await?;
        Ok(transactions)
    }

    /// Excludes reorganized transactions - preferred for user-facing display.
    pub async fn load_transactions_excluding_reorged(
        &self,
        account_id: Id,
    ) -> Result<Vec<DisplayedTransaction>, TransactionHistoryError> {
        let mut conn = self.db_pool.acquire().await?;
        let transactions = db::get_displayed_transactions_excluding_reorged(&mut conn, account_id).await?;
        Ok(transactions)
    }

    pub async fn load_transactions_by_status(
        &self,
        account_id: Id,
        status: TransactionDisplayStatus,
    ) -> Result<Vec<DisplayedTransaction>, TransactionHistoryError> {
        let mut conn = self.db_pool.acquire().await?;
        let transactions = db::get_displayed_transactions_by_status(&mut conn, account_id, status).await?;
        Ok(transactions)
    }

    pub async fn load_transaction_by_id(
        &self,
        id: &str,
    ) -> Result<Option<DisplayedTransaction>, TransactionHistoryError> {
        let mut conn = self.db_pool.acquire().await?;
        let transaction = db::get_displayed_transaction_by_id(&mut conn, id).await?;
        Ok(transaction)
    }

    pub async fn load_transactions_paginated(
        &self,
        account_id: Id,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<DisplayedTransaction>, TransactionHistoryError> {
        let mut conn = self.db_pool.acquire().await?;
        let transactions = db::get_displayed_transactions_paginated(&mut conn, account_id, limit, offset).await?;
        Ok(transactions)
    }

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

    /// Fallback for legacy data migration.
    #[allow(dead_code)]
    pub async fn rebuild_from_balance_changes(
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
}
