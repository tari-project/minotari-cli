use anyhow::anyhow;
use lightweight_wallet_libs::{HttpBlockchainScanner, scanning::BlockchainScanner};
use sqlx::{Acquire, SqliteConnection};
use tari_common_types::types::FixedHash;
use tari_transaction_components::key_manager::{
    TransactionKeyManagerWrapper, memory_key_manager::MemoryKeyManagerBackend,
};

use crate::{
    db,
    models::{WalletEvent, WalletEventType},
};

pub async fn handle_reorgs(
    scanner: &mut HttpBlockchainScanner<TransactionKeyManagerWrapper<MemoryKeyManagerBackend>>,
    conn: &mut SqliteConnection,
    account_id: i64,
) -> Result<u64, anyhow::Error> {
    let last_blocks = db::get_scanned_tip_blocks_by_account(conn, account_id).await?;

    if last_blocks.is_empty() {
        return Ok(0); // No blocks scanned yet, start from genesis.
    }

    let mut reorg_start_height = 0;
    let mut reorged_blocks = vec![];

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
                reorged_blocks.push(block);
            }
        } else {
            // Block no longer exists on chain, it's reorged.
            reorged_blocks.push(block);
        }
    }

    if reorg_start_height == 0 && !reorged_blocks.is_empty() {
        // All scanned blocks were reorged, rescan from birthday.
        println!("All previously scanned blocks have been reorged, starting from genesis.");
        reorg_start_height = 0; // Indicate full rescan
    } else if !reorged_blocks.is_empty() {
        println!("REORG DETECTED. Rolling back from height: {}", reorg_start_height);
        let mut tx = conn.begin().await?;
        rollback_from_height(&mut tx, account_id, reorg_start_height).await?;
        tx.commit().await?;
    } else {
        println!("No reorgs detected.");
        reorg_start_height = last_blocks[0].height + 1;
    }

    Ok(reorg_start_height)
}

async fn rollback_from_height(
    tx: &mut SqliteConnection,
    account_id: i64,
    reorg_start_height: u64,
) -> Result<(), anyhow::Error> {
    // 1. Log BlockRolledBack events
    let rolled_back_blocks = db::get_scanned_tip_blocks_by_account(tx, account_id)
        .await?
        .into_iter()
        .filter(|b| b.height >= reorg_start_height)
        .collect::<Vec<_>>();

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
        SELECT output_hash, mined_in_block_height
        FROM outputs
        WHERE account_id = ? AND mined_in_block_height >= ?
        "#,
        account_id,
        output_reorg_start_height,
    )
    .fetch_all(&mut *tx)
    .await?;

    for output_row in affected_outputs {
        let event = WalletEvent {
            id: 0,
            account_id,
            event_type: WalletEventType::OutputRolledBack {
                hash: FixedHash::try_from(output_row.output_hash.as_slice())?,
                original_block_height: output_row.mined_in_block_height as u64,
                rolled_back_at_height: reorg_start_height,
            },
            description: format!(
                "Output {:x?} at height {} was rolled back due to reorg",
                output_row.output_hash, output_row.mined_in_block_height
            ),
        };
        db::insert_wallet_event(tx, account_id, &event).await?;
    }

    // 3. Revert output statuses (e.g., SPENT or LOCKED back to UNSPENT)
    db::update_output_status_to_unspent_from_height(tx, account_id, reorg_start_height).await?;

    // 4. Delete dependent data in correct order
    db::delete_balance_changes_from_height(tx, account_id, reorg_start_height).await?;
    db::delete_inputs_from_height(tx, account_id, reorg_start_height).await?;
    db::delete_outputs_from_height(tx, account_id, reorg_start_height).await?;
    db::delete_scanned_tip_blocks_from_height(tx, account_id, reorg_start_height).await?;

    Ok(())
}
