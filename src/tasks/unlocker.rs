use std::time::Duration;

use rusqlite::Connection;
use tokio::{sync::broadcast, task::JoinHandle, time::interval};

use crate::{
    db::{self, SqlitePool},
    models::PendingTransactionStatus,
};

pub struct TransactionUnlocker {
    db_pool: SqlitePool,
}

impl TransactionUnlocker {
    pub fn new(db_pool: SqlitePool) -> Self {
        Self { db_pool }
    }

    pub fn unlock_expired_transactions(conn: &mut Connection) -> Result<(), anyhow::Error> {
        let expired_txs = db::find_expired_pending_transactions(conn)?;

        for tx in expired_txs {
            let transaction = conn.transaction()?;

            db::update_pending_transaction_status(&transaction, &tx.id, PendingTransactionStatus::Expired)?;
            db::unlock_outputs_for_request(&transaction, &tx.id)?;

            transaction.commit()?;
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
                        let mut conn = self.db_pool.get()?;
                        if let Err(e) = Self::unlock_expired_transactions(&mut conn) {
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
