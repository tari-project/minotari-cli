use log::{debug, error};
use r2d2::PooledConnection;
use r2d2_sqlite::SqliteConnectionManager;

use crate::{
    ScanError, WalletEvent,
    db::{SqlitePool, WalletDbError, prune_scanned_tip_blocks},
    scan::{BlockProcessor, EventSender},
};

#[derive(Clone)]
pub struct ScanDbHandler {
    pool: SqlitePool,
}

impl ScanDbHandler {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn get_connection(&self) -> Result<PooledConnection<SqliteConnectionManager>, ScanError> {
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || pool.get().map_err(WalletDbError::from))
            .await
            .map_err(|e| {
                let err = anyhow::anyhow!("DB connection task failed: {}", e);
                error!("DB connection task failed: {}", e);
                ScanError::Fatal(err)
            })?
            .map_err(ScanError::DbError)
    }

    pub async fn process_blocks<E: EventSender + Clone + Send + 'static>(
        &self,
        blocks: Vec<lightweight_wallet_libs::BlockScanResult>,
        account_id: i64,
        view_key: Vec<u8>,
        event_sender: E,
        has_pending_outbound: bool,
    ) -> Result<Vec<WalletEvent>, ScanError> {
        if blocks.is_empty() {
            return Ok(Vec::new());
        }

        debug!(
            count = blocks.len(),
            account_id = account_id;
            "Processing scanned blocks in DB task"
        );

        let pool = self.pool.clone();

        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(WalletDbError::from)?;
            let mut events = Vec::new();

            for block in blocks {
                let tx = conn.transaction().map_err(WalletDbError::from)?;

                let mut processor = BlockProcessor::with_event_sender(
                    account_id,
                    view_key.clone(),
                    event_sender.clone(),
                    has_pending_outbound,
                );

                processor
                    .process_block(&tx, &block)
                    .map_err(|e| WalletDbError::Unexpected(e.to_string()))?;

                events.extend(processor.into_wallet_events());

                tx.commit().map_err(WalletDbError::from)?;
            }
            Ok::<Vec<WalletEvent>, WalletDbError>(events)
        })
        .await
        .map_err(|e| {
            let err = anyhow::anyhow!("Block processing task failed: {}", e);
            error!("Block processing task failed: {}", e);
            ScanError::Fatal(err)
        })?
        .map_err(ScanError::from)
    }

    pub async fn prune_tips(&self, account_id: i64, height: u64) -> Result<(), ScanError> {
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || {
            let conn = pool.get().map_err(WalletDbError::from)?;
            prune_scanned_tip_blocks(&conn, account_id, height)
        })
        .await
        .map_err(|e| {
            let err = anyhow::anyhow!("Pruning task failed: {}", e);
            error!("Pruning task failed: {}", e);
            ScanError::Fatal(err)
        })?
        .map_err(ScanError::DbError)
    }
}
