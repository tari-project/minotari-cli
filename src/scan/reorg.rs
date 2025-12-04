use crate::{
    db,
    models::{PendingTransactionStatus, WalletEvent, WalletEventType},
    transactions::DisplayedTransaction,
};
use anyhow::anyhow;
use lightweight_wallet_libs::{HttpBlockchainScanner, scanning::BlockchainScanner};
use sqlx::{Acquire, SqliteConnection};
use std::collections::HashSet;
use tari_common_types::types::FixedHash;
use tari_transaction_components::key_manager::KeyManager;

/// Result of a reorg check operation.
#[derive(Debug, Clone)]
pub struct ReorgResult {
    pub resume_height: u64,
    pub reorg_information: Option<ReorgInformation>,
}

/// Details about a detected reorg.
#[derive(Debug, Clone)]
pub struct ReorgInformation {
    pub rolled_back_from_height: u64,
    pub rolled_back_blocks_count: u64,
    pub invalidated_output_hashes: Vec<FixedHash>,
    pub cancelled_transaction_ids: Vec<String>,
    pub reorganized_displayed_transactions: Vec<DisplayedTransaction>,
}

pub async fn handle_reorgs(
    scanner: &mut HttpBlockchainScanner<KeyManager>,
    conn: &mut SqliteConnection,
    account_id: i64,
) -> Result<ReorgResult, anyhow::Error> {
    let last_blocks = db::get_scanned_tip_blocks_by_account(conn, account_id).await?;

    if last_blocks.is_empty() {
        return Ok(ReorgResult {
            resume_height: 0,
            reorg_information: None,
        });
    }

    let mut reorg_start_height = 0;
    let mut is_reorg_detected = false;

    for block in &last_blocks {
        let chain_block = scanner
            .get_header_by_height(block.height)
            .await
            .map_err(|e| anyhow!("Failed to get header by height: {}", e))?;

        if let Some(chain_block) = chain_block {
            if chain_block.hash == block.hash {
                // Found the fork point. All blocks after this height are reorged.
                reorg_start_height = block.height + 1;
                break;
            } else {
                // This block is reorged.
                is_reorg_detected = true;
            }
        } else {
            // Block no longer exists on chain, it's reorged.
            is_reorg_detected = true;
        }
    }

    if is_reorg_detected {
        println!("REORG DETECTED. Rolling back from height: {}", reorg_start_height);
        let mut tx = conn.begin().await?;
        let reorg_info = rollback_from_height(&mut tx, account_id, reorg_start_height).await?;
        tx.commit().await?;
        Ok(ReorgResult {
            resume_height: reorg_start_height,
            reorg_information: Some(reorg_info),
        })
    } else {
        println!("No reorgs detected.");
        Ok(ReorgResult {
            resume_height: last_blocks[0].height + 1,
            reorg_information: None,
        })
    }
}

async fn rollback_from_height(
    tx: &mut SqliteConnection,
    account_id: i64,
    reorg_start_height: u64,
) -> Result<ReorgInformation, anyhow::Error> {
    // 1. Log BlockRolledBack events
    let rolled_back_blocks = db::get_scanned_tip_blocks_by_account(tx, account_id)
        .await?
        .into_iter()
        .filter(|b| b.height >= reorg_start_height)
        .collect::<Vec<_>>();

    let blocks_rolled_back = rolled_back_blocks.len() as u64;

    for block in rolled_back_blocks {
        let event = WalletEvent {
            id: 0,
            account_id,
            event_type: WalletEventType::BlockRolledBack {
                height: block.height,
                block_hash: block.hash,
            },
            description: format!("Block at height {} was rolled back due to reorg", block.height),
        };
        db::insert_wallet_event(tx, account_id, &event).await?;
    }

    // 2. Get outputs that will be affected to log OutputRolledBack events
    let output_reorg_start_height = reorg_start_height as i64;
    let affected_outputs = sqlx::query!(
        r#"
        SELECT output_hash, mined_in_block_height, locked_by_request_id
        FROM outputs
        WHERE account_id = ? AND mined_in_block_height >= ? AND deleted_at IS NULL
        "#,
        account_id,
        output_reorg_start_height,
    )
    .fetch_all(&mut *tx)
    .await?;

    let mut cancelled_pending_tx_ids = HashSet::new();
    let mut invalidated_output_hashes = Vec::new();

    for output_row in &affected_outputs {
        let output_hash = FixedHash::try_from(output_row.output_hash.as_slice())?;
        invalidated_output_hashes.push(output_hash);

        let event = WalletEvent {
            id: 0,
            account_id,
            event_type: WalletEventType::OutputRolledBack {
                hash: output_hash,
                original_block_height: output_row.mined_in_block_height as u64,
                rolled_back_at_height: reorg_start_height,
            },
            description: format!(
                "Output {:x?} at height {} was rolled back due to reorg",
                output_row.output_hash, output_row.mined_in_block_height
            ),
        };
        db::insert_wallet_event(tx, account_id, &event).await?;

        if let Some(request_id) = &output_row.locked_by_request_id {
            cancelled_pending_tx_ids.insert(request_id.clone());
        }
    }

    // 3. Cancel pending transactions linked to reorged outputs
    let cancelled_transaction_ids: Vec<String> = cancelled_pending_tx_ids.into_iter().collect();
    if !cancelled_transaction_ids.is_empty() {
        db::cancel_pending_transactions_by_ids(tx, &cancelled_transaction_ids, PendingTransactionStatus::Cancelled)
            .await?;

        for tx_id in &cancelled_transaction_ids {
            let event = WalletEvent {
                id: 0,
                account_id,
                event_type: WalletEventType::PendingTransactionCancelled {
                    tx_id: tx_id.clone(),
                    reason: "Transaction cancelled due to blockchain reorg".to_string(),
                },
                description: format!("Pending transaction {} cancelled due to reorg", tx_id),
            };
            db::insert_wallet_event(tx, account_id, &event).await?;
        }
    }

    // 4. Soft delete dependent data and create reversal balance changes
    db::soft_delete_inputs_from_height(tx, account_id, reorg_start_height).await?;
    db::soft_delete_outputs_from_height(tx, account_id, reorg_start_height).await?;
    db::delete_scanned_tip_blocks_from_height(tx, account_id, reorg_start_height).await?;

    let reorganized_displayed_transactions =
        db::mark_displayed_transactions_reorganized_and_return(tx, account_id, reorg_start_height).await?;

    let affected_count = db::reset_mined_completed_transactions_from_height(tx, account_id, reorg_start_height).await?;
    if affected_count > 0 {
        println!(
            "Reset {} completed transaction(s) to Completed status due to reorg",
            affected_count
        );
    }

    Ok(ReorgInformation {
        rolled_back_from_height: reorg_start_height,
        rolled_back_blocks_count: blocks_rolled_back,
        invalidated_output_hashes,
        cancelled_transaction_ids,
        reorganized_displayed_transactions,
    })
}
