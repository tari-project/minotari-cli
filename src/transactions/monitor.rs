use anyhow::{Result, anyhow};
use sqlx::SqliteConnection;

use crate::db::{
    self, CompletedTransaction, CompletedTransactionStatus, get_completed_transactions_by_status,
    mark_completed_transaction_as_broadcasted, mark_completed_transaction_as_confirmed,
    mark_completed_transaction_as_mined_unconfirmed, mark_completed_transaction_as_rejected,
    revert_completed_transaction_to_completed,
};
use crate::http::{TxLocation, TxSubmissionRejectionReason, WalletHttpClient};
use crate::models::{WalletEvent, WalletEventType};
use lightweight_wallet_libs::HttpBlockchainScanner;
use tari_transaction_components::key_manager::KeyManager;
use tari_transaction_components::transaction_components::Transaction;
use tari_utilities::ByteArray;

const REQUIRED_CONFIRMATIONS: u64 = 6;
const MAX_BROADCAST_ATTEMPTS: i32 = 10;

pub struct TransactionMonitor;

impl TransactionMonitor {
    pub async fn monitor_transactions(
        scanner: &mut HttpBlockchainScanner<KeyManager>,
        wallet_client: &WalletHttpClient,
        conn: &mut SqliteConnection,
        account_id: i64,
        current_chain_height: u64,
    ) -> Result<Vec<WalletEvent>> {
        let mut events = Vec::new();

        events.extend(Self::rebroadcast_completed_transactions(wallet_client, conn, account_id).await?);
        events.extend(Self::check_broadcast_for_mining(wallet_client, conn, account_id).await?);
        events.extend(Self::check_confirmation_status(scanner, conn, account_id, current_chain_height).await?);

        Ok(events)
    }

    async fn rebroadcast_completed_transactions(
        wallet_client: &WalletHttpClient,
        conn: &mut SqliteConnection,
        account_id: i64,
    ) -> Result<Vec<WalletEvent>> {
        let mut events = Vec::new();

        let completed_txs =
            get_completed_transactions_by_status(conn, account_id, CompletedTransactionStatus::Completed).await?;

        for tx in completed_txs {
            if tx.broadcast_attempts >= MAX_BROADCAST_ATTEMPTS {
                let reason = format!("Exceeded {} broadcast attempts", MAX_BROADCAST_ATTEMPTS);
                mark_completed_transaction_as_rejected(conn, &tx.id, &reason).await?;
                db::unlock_outputs_for_pending_transaction(conn, &tx.pending_tx_id).await?;

                events.push(WalletEvent {
                    id: 0,
                    account_id,
                    event_type: WalletEventType::TransactionRejected {
                        tx_id: tx.id.clone(),
                        reason,
                    },
                    description: format!("Transaction {} exceeded broadcast attempts", tx.id),
                });
                continue;
            }

            match Self::broadcast_transaction(wallet_client, &tx).await {
                Ok(()) => {
                    mark_completed_transaction_as_broadcasted(conn, &tx.id, tx.broadcast_attempts + 1).await?;

                    events.push(WalletEvent {
                        id: 0,
                        account_id,
                        event_type: WalletEventType::TransactionBroadcast {
                            tx_id: tx.id.clone(),
                            kernel_excess: tx.kernel_excess.clone(),
                        },
                        description: format!("Transaction {} broadcast", tx.id),
                    });
                },
                Err(reason) => {
                    mark_completed_transaction_as_rejected(conn, &tx.id, &reason).await?;
                    db::unlock_outputs_for_pending_transaction(conn, &tx.pending_tx_id).await?;

                    events.push(WalletEvent {
                        id: 0,
                        account_id,
                        event_type: WalletEventType::TransactionRejected {
                            tx_id: tx.id.clone(),
                            reason,
                        },
                        description: format!("Transaction {} rejected", tx.id),
                    });
                },
            }
        }

        Ok(events)
    }

    async fn check_broadcast_for_mining(
        wallet_client: &WalletHttpClient,
        conn: &mut SqliteConnection,
        account_id: i64,
    ) -> Result<Vec<WalletEvent>> {
        let mut events = Vec::new();

        let broadcast_txs =
            get_completed_transactions_by_status(conn, account_id, CompletedTransactionStatus::Broadcast).await?;

        for tx in broadcast_txs {
            if let Some((block_height, block_hash)) = Self::find_kernel_on_chain(wallet_client, &tx).await? {
                mark_completed_transaction_as_mined_unconfirmed(conn, &tx.id, block_height as i64, &block_hash).await?;

                events.push(WalletEvent {
                    id: 0,
                    account_id,
                    event_type: WalletEventType::TransactionUnconfirmed {
                        tx_id: tx.id.clone(),
                        mined_height: block_height,
                        confirmations: 0,
                    },
                    description: format!("Transaction {} mined at height {}", tx.id, block_height),
                });
            }
        }

        Ok(events)
    }

    async fn check_confirmation_status(
        scanner: &mut HttpBlockchainScanner<KeyManager>,
        conn: &mut SqliteConnection,
        account_id: i64,
        current_height: u64,
    ) -> Result<Vec<WalletEvent>> {
        let mut events = Vec::new();

        let unconfirmed_txs =
            get_completed_transactions_by_status(conn, account_id, CompletedTransactionStatus::MinedUnconfirmed)
                .await?;

        for tx in unconfirmed_txs {
            let mined_height = match tx.mined_height {
                Some(h) => h as u64,
                None => continue,
            };

            let is_on_main_chain =
                Self::verify_block_on_chain(scanner, mined_height, tx.mined_block_hash.as_ref()).await?;

            if !is_on_main_chain {
                revert_completed_transaction_to_completed(conn, &tx.id).await?;

                events.push(WalletEvent {
                    id: 0,
                    account_id,
                    event_type: WalletEventType::TransactionReorged {
                        tx_id: tx.id.clone(),
                        original_mined_height: mined_height,
                    },
                    description: format!("Transaction {} reorged from height {}", tx.id, mined_height),
                });
                continue;
            }

            let confirmations = current_height.saturating_sub(mined_height);
            if confirmations >= REQUIRED_CONFIRMATIONS {
                mark_completed_transaction_as_confirmed(conn, &tx.id, current_height as i64).await?;

                events.push(WalletEvent {
                    id: 0,
                    account_id,
                    event_type: WalletEventType::TransactionConfirmed {
                        tx_id: tx.id.clone(),
                        mined_height,
                        confirmation_height: current_height,
                    },
                    description: format!("Transaction {} confirmed", tx.id),
                });
            }
        }

        Ok(events)
    }

    async fn broadcast_transaction(wallet_client: &WalletHttpClient, tx: &CompletedTransaction) -> Result<(), String> {
        let transaction: Transaction =
            serde_json::from_slice(&tx.serialized_transaction).map_err(|e| format!("Deserialization failed: {}", e))?;

        let response = wallet_client
            .submit_transaction(transaction)
            .await
            .map_err(|e| format!("Broadcast failed: {}", e))?;

        if response.accepted || response.rejection_reason == TxSubmissionRejectionReason::AlreadyMined {
            Ok(())
        } else {
            Err(format!("Transaction rejected: {}", response.rejection_reason))
        }
    }

    async fn find_kernel_on_chain(
        wallet_client: &WalletHttpClient,
        tx: &CompletedTransaction,
    ) -> Result<Option<(u64, Vec<u8>)>> {
        let transaction: Transaction =
            serde_json::from_slice(&tx.serialized_transaction).map_err(|e| anyhow!("Deserialization failed: {}", e))?;

        let kernel = transaction
            .body()
            .kernels()
            .first()
            .ok_or_else(|| anyhow!("Transaction has no kernel"))?;

        let excess_sig_nonce = kernel.excess_sig.get_compressed_public_nonce().as_bytes();
        let excess_sig = kernel.excess_sig.get_signature().as_bytes();

        let response = wallet_client
            .transaction_query(excess_sig_nonce, excess_sig)
            .await
            .map_err(|e| anyhow!("Transaction query failed: {}", e))?;

        match response.location {
            TxLocation::Mined => {
                let height = response
                    .mined_height
                    .ok_or_else(|| anyhow!("Mined transaction missing height"))?;
                let hash = response
                    .mined_header_hash
                    .ok_or_else(|| anyhow!("Mined transaction missing block hash"))?;
                Ok(Some((height, hash)))
            },
            _ => Ok(None),
        }
    }

    async fn verify_block_on_chain(
        scanner: &mut HttpBlockchainScanner<KeyManager>,
        height: u64,
        expected_hash: Option<&Vec<u8>>,
    ) -> Result<bool> {
        use lightweight_wallet_libs::scanning::BlockchainScanner;

        let expected = match expected_hash {
            Some(h) => h,
            None => return Ok(false),
        };

        match scanner.get_header_by_height(height).await {
            Ok(Some(header)) => Ok(&header.hash == expected),
            Ok(None) => Ok(false),
            Err(e) => Err(anyhow!("Failed to verify block: {}", e)),
        }
    }
}
