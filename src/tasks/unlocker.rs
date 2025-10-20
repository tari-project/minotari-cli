use std::time::Duration;

use sqlx::{Connection, SqliteConnection, SqlitePool};
use tokio::{sync::broadcast, task::JoinHandle, time::interval};

use crate::db;

pub struct TransactionUnlocker {
    db_pool: SqlitePool,
}

impl TransactionUnlocker {
    pub fn new(db_pool: SqlitePool) -> Self {
        Self { db_pool }
    }

    pub async fn unlock_expired_transactions(conn: &mut SqliteConnection) -> Result<(), anyhow::Error> {
        let expired_txs = db::find_expired_pending_transactions(conn).await?;

        for tx in expired_txs {
            let mut transaction = conn.begin().await?;

            db::update_pending_transaction_status(&mut transaction, &tx.id, "EXPIRED").await?;
            db::unlock_outputs_for_request(&mut transaction, &tx.id).await?;

            transaction.commit().await?;
        }

        Ok(())
    }

    pub fn run(self, mut shutdown_rx: broadcast::Receiver<()>) -> JoinHandle<Result<(), anyhow::Error>> {
        tokio::spawn(async move {
            println!("Transaction unlocker task started.");
            let mut interval = interval(Duration::from_secs(60));

            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        let mut conn = self.db_pool.acquire().await?;
                        if let Err(e) = Self::unlock_expired_transactions(&mut conn).await {
                            eprintln!("Error unlocking expired transactions: {}", e);
                        }
                    }
                    _ = shutdown_rx.recv() => {
                        println!("Transaction unlocker task received shutdown signal. Exiting gracefully.");
                        break;
                    }
                }
            }
            println!("Transaction unlocker task has shut down.");
            Ok(())
        })
    }
}
