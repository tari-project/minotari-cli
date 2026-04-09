use crate::{
    db::{self, get_active_outputs_from_height},
    models::{PendingTransactionStatus, WalletEvent, WalletEventType},
    transactions::DisplayedTransaction,
    webhooks::{WebhookTriggerConfig, utils::trigger_webhook_with_balance},
    wallet_db_extensions, // New import for payref tracking
};
use anyhow::anyhow;
use log::{debug, info, warn};
use minotari_scanning::{HttpBlockchainScanner, scanning::BlockchainScanner};
use rusqlite::Connection;
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
    conn: &mut Connection,
    account_id: i64,
    webhook_config: Option<WebhookTriggerConfig>,
) -> Result<ReorgResult, anyhow::Error> {
    let last_blocks = db::get_scanned_tip_blocks_by_account(conn, account_id)?;

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
        warn!(
            target: "audit",
            account_id = account_id,
            rollback_height = reorg_start_height;
            "REORG DETECTED. Rolling back chain state."
        );
        let tx = conn.transaction()?;
        let reorg_info = rollback_from_height(&tx, account_id, reorg_start_height, webhook_config)?;
        tx.commit()?;
        Ok(ReorgResult {
            resume_height: reorg_start_height,
            reorg_information: Some(reorg_info),
        })
    } else {
        debug!("No reorgs detected.");
        Ok(ReorgResult {
            resume_height: last_blocks.first().expect("is already checked").height + 1,
            reorg_information: None,
        })
    }
}

pub fn rollback_from_height(
    tx: &Connection,
    account_id: i64,
    reorg_start_height: u64,
    webhook_config: Option<WebhookTriggerConfig>,
) -> Result<ReorgInformation, anyhow::Error> {
    let mut generated_events: Vec<(i64, WalletEvent)> = Vec::new();

    // 1. Log BlockRolledBack events
    let rolled_back_blocks = db::get_scanned_tip_blocks_by_account(tx, account_id)?
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
        db::insert_wallet_event(tx, &event)?;
        generated_events.push((account_id, event));
    }

    // 2. Invalidate affected outputs
    let invalidated_output_hashes = get_active_outputs_from_height(tx, account_id, reorg_start_height)?;
    for output_hash in &invalidated_output_hashes {
        db::mark_output_as_invalidated(tx, output_hash)?;
        let event = WalletEvent {
            id: 0,
            account_id,
            event_type: WalletEventType::OutputInvalidated {
                output_hash: *output_hash,
            },
            description: format!("Output {} invalidated due to reorg", output_hash),
        };
        db::insert_wallet_event(tx, &event)?;
        generated_events.push((account_id, event));
    }

    // 3. Mark pending transactions as cancelled if their outputs are invalidated
    let cancelled_transaction_ids =
        db::get_pending_transactions_by_output_hashes(tx, account_id, &invalidated_output_hashes)?;

    for tx_id in &cancelled_transaction_ids {
        db::update_pending_transaction_status(tx, *tx_id, PendingTransactionStatus::Cancelled)?;
        let event = WalletEvent {
            id: 0,
            account_id,
            event_type: WalletEventType::PendingTransactionCancelled {
                transaction_id: *tx_id,
            },
            description: format!("Pending transaction {} cancelled due to reorg", tx_id),
        };
        db::insert_wallet_event(tx, &event)?;
        generated_events.push((account_id, event));
    }

    // 4. Update completed transactions that were in reorged blocks and store old payrefs
    let reorganized_displayed_transactions =
        db::get_displayed_transactions_above_height(tx, account_id, reorg_start_height)?;

    for tx_to_reorg in &reorganized_displayed_transactions {
        // If a completed transaction had a payref and is now being reorged,
        // its payref might change if it gets re-mined. Store the old payref.
        if let Some(old_payref) = tx_to_reorg.payref {
            for output in &tx_to_reorg.outputs {
                wallet_db_extensions::insert_reorg_payref_entry(
                    tx,
                    tx_to_reorg.transaction_id,
                    output.output_hash,
                    old_payref,
                )?;
            }
        }
        // Mark the completed transaction as unconfirmed so it can be re-scanned
        // and re-confirmed, potentially with a new block hash and thus a new payref.
        db::mark_completed_transaction_as_unconfirmed(tx, tx_to_reorg.transaction_id)?;
        let event = WalletEvent {
            id: 0,
            account_id,
            event_type: WalletEventType::CompletedTransactionReorged {
                transaction_id: tx_to_reorg.transaction_id,
                payref: tx_to_reorg.payref,
            },
            description: format!(
                "Completed transaction {} (payref: {:?}) reorged",
                tx_to_reorg.transaction_id, tx_to_reorg.payref
            ),
        };
        db::insert_wallet_event(tx, &event)?;
        generated_events.push((account_id, event));
    }

    // 5. Delete scanned tip blocks above reorg_start_height
    db::delete_scanned_tip_blocks_above_height(tx, account_id, reorg_start_height)?;

    // 6. Trigger webhook for balance update if configured
    if let Some(webhook_cfg) = webhook_config {
        trigger_webhook_with_balance(tx, account_id, webhook_cfg, &generated_events)?;
    }

    Ok(ReorgInformation {
        rolled_back_from_height: reorg_start_height,
        rolled_back_blocks_count: blocks_rolled_back,
        invalidated_output_hashes,
        cancelled_transaction_ids: cancelled_transaction_ids
            .into_iter()
            .map(|id| id.to_string())
            .collect(),
        reorganized_displayed_transactions,
    })
}};
        let event_id = db::insert_wallet_event(tx, account_id, &event)?;
        generated_events.push((event_id, event));
    }

    // 2. Get outputs that will be affected to log OutputRolledBack events
    let affected_outputs = get_active_outputs_from_height(tx, account_id, reorg_start_height)?;

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
        let event_id = db::insert_wallet_event(tx, account_id, &event)?;
        generated_events.push((event_id, event));

        if let Some(request_id) = &output_row.locked_by_request_id {
            cancelled_pending_tx_ids.insert(request_id.clone());
        }
    }

    // 3. Cancel pending transactions linked to reorged outputs
    let cancelled_transaction_ids: Vec<String> = cancelled_pending_tx_ids.into_iter().collect();
    if !cancelled_transaction_ids.is_empty() {
        db::cancel_pending_transactions_by_ids(tx, &cancelled_transaction_ids, PendingTransactionStatus::Cancelled)?;

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
            let event_id = db::insert_wallet_event(tx, account_id, &event)?;
            generated_events.push((event_id, event));
        }
    }

    // 4. Soft delete dependent data and create reversal balance changes
    db::soft_delete_inputs_from_height(tx, account_id, reorg_start_height)?;
    db::soft_delete_outputs_from_height(tx, account_id, reorg_start_height)?;
    db::delete_scanned_tip_blocks_from_height(tx, account_id, reorg_start_height)?;

    let reorganized_displayed_transactions =
        db::mark_displayed_transactions_reorganized_and_return(tx, account_id, reorg_start_height)?;

    let affected_count = db::reset_mined_completed_transactions_from_height(tx, account_id, reorg_start_height)?;
    if affected_count > 0 {
        info!(
            target: "audit",
            count = affected_count;
            "Reset completed transaction(s) to Completed status due to reorg"
        );
    }

    if let Some(config) = webhook_config {
        for (event_id, event) in generated_events {
            trigger_webhook_with_balance(tx, account_id, event_id, &event, &config)?;
        }
    }

    Ok(ReorgInformation {
        rolled_back_from_height: reorg_start_height,
        rolled_back_blocks_count: blocks_rolled_back,
        invalidated_output_hashes,
        cancelled_transaction_ids,
        reorganized_displayed_transactions,
    })
}
