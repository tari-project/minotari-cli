use chrono::{DateTime, Utc};
use lightweight_wallet_libs::{HttpBlockchainScanner, ScanConfig, scanning::BlockchainScanner};
use sqlx::Acquire;
use std::time::Instant;

use tari_transaction_components::transaction_components::WalletOutput;
use tari_utilities::byte_array::ByteArray;
use thiserror::Error;

use crate::db::AccountTypeRow;
use crate::{
    db,
    models::{self, BalanceChange, WalletEvent},
    reorg,
};

const BIRTHDAY_GENESIS_FROM_UNIX_EPOCH: u64 = 1640995200;
const MAINNET_GENESIS_DATE: u64 = 1746489644; // 6 May 2025

#[derive(Debug, Error)]
pub enum ScanError {
    #[error("Fatal error: {0}")]
    Fatal(#[from] anyhow::Error),
    #[error("Fatal error: {0}")]
    FatalSqlx(#[from] sqlx::Error),
    #[error("Intermittent error: {0}")]
    Intermittent(String),
}

pub async fn scan(
    password: &str,
    base_url: &str,
    database_file: &str,
    account_name: Option<&str>,
    max_blocks: u64,
    batch_size: u64,
) -> Result<Vec<WalletEvent>, ScanError> {
    let pool = db::init_db(database_file).await.map_err(ScanError::Fatal)?;
    let mut conn = pool.acquire().await?;
    let mut result = vec![];
    for account in db::get_accounts(&mut conn, account_name, true)
        .await
        .map_err(ScanError::FatalSqlx)?
    {
        let account_type_row = if account.is_parent() {
            let a = account.try_into_parent().map_err(ScanError::Fatal)?;
            AccountTypeRow::from_parent(a)
        } else {
            if account.parent_account_id.is_none() {
                return Err(ScanError::Fatal(anyhow::anyhow!(
                    "Child account found but parent_account_id is None"
                )));
            }
            let parent_account_id = account.parent_account_id.unwrap();
            let a = db::get_account_by_id(&mut conn, parent_account_id)
                .await
                .map_err(ScanError::FatalSqlx)?
                .ok_or_else(|| ScanError::Fatal(anyhow::anyhow!("Parent account not found")))?
                .try_into_parent()?;
            let child = account.try_into_child(a)?;

            AccountTypeRow::from_child(child)
        };

        let key_manager = account_type_row
            .get_key_manager(password)
            .await
            .map_err(ScanError::Fatal)?;
        let mut scanner = HttpBlockchainScanner::new(base_url.to_string(), key_manager.clone(), 8)
            .await
            .map_err(|e| ScanError::Intermittent(e.to_string()))?;

        let mut start_height = reorg::handle_reorgs(&mut scanner, &mut conn, account_type_row.account_id())
            .await
            .map_err(ScanError::Fatal)?;

        if start_height == 0 {
            let birthday_day = (account_type_row.birthday() as u64) * 24 * 60 * 60 + BIRTHDAY_GENESIS_FROM_UNIX_EPOCH;
            println!(
                "Calculating birthday day from birthday {} to be {} (unix epoch)",
                account_type_row.birthday(),
                birthday_day
            );
            let estimate_birthday_block = (birthday_day.saturating_sub(MAINNET_GENESIS_DATE)) / 120; // 2 minute blocks
            println!(
                "Estimating birthday block height from birthday {} to be {}",
                account_type_row.birthday(),
                estimate_birthday_block
            );
            start_height = estimate_birthday_block;
        }

        let mut total_scanned = 0;
        let total_timer = Instant::now();
        let scan_config = ScanConfig::default()
            .with_start_height(start_height)
            .with_batch_size(batch_size);
        println!("starting scan from height {}", start_height);
        let mut scan_update_height = start_height + 1000;
        loop {
            if total_scanned >= max_blocks {
                println!("Reached maximum number of blocks to scan: {}", max_blocks);
                break;
            }
            let (scanned_blocks, more_blocks) = scanner
                .scan_blocks(&scan_config)
                .await
                .map_err(|e| ScanError::Intermittent(e.to_string()))?;

            total_scanned += scanned_blocks.len() as u64;
            if scanned_blocks.is_empty() || !more_blocks {
                println!("No more blocks to scan.");
                break;
            }
            for scanned_block in &scanned_blocks {
                if scanned_block.height >= scan_update_height {
                    println!("Scanned {} blocks so far...", total_scanned);
                    println!("Total scan time so far: {:?}", total_timer.elapsed());
                    scan_update_height += 1000;
                }
                // Start a transaction for all DB operations related to this scanned block so
                // that either all inserts/updates succeed or none are applied.
                let mut tx = conn.begin().await.map_err(ScanError::FatalSqlx)?;
                // Deleted inputs
                for input in &scanned_block.inputs {
                    if let Some((output_id, value)) = db::get_output_info_by_hash(&mut tx, input.as_slice())
                        .await
                        .map_err(ScanError::FatalSqlx)?
                    {
                        let (input_id, inserted_new_input) = db::insert_input(
                            &mut tx,
                            account_type_row.account_id(),
                            output_id,
                            scanned_block.height,
                            scanned_block.block_hash.as_slice(),
                            scanned_block.mined_timestamp,
                        )
                        .await
                        .map_err(ScanError::FatalSqlx)?;

                        if inserted_new_input {
                            let balance_change = BalanceChange {
                                account_id: account_type_row.account_id(),
                                caused_by_output_id: None,
                                caused_by_input_id: Some(input_id),
                                description: "Output spent as input".to_string(),
                                balance_credit: 0,
                                balance_debit: value,
                                effective_date: DateTime::<Utc>::from_timestamp(
                                    scanned_block.mined_timestamp as i64,
                                    0,
                                )
                                .unwrap()
                                .naive_utc(),
                                effective_height: scanned_block.height,
                                claimed_recipient_address: None,
                                claimed_sender_address: None,
                                memo_hex: None,
                                memo_parsed: None,
                                claimed_fee: None,
                                claimed_amount: None,
                            };
                            db::insert_balance_change(&mut tx, &balance_change)
                                .await
                                .map_err(ScanError::FatalSqlx)?;

                            db::update_output_status(&mut tx, output_id, models::OutputStatus::Spent)
                                .await
                                .map_err(ScanError::FatalSqlx)?;
                        }
                    }
                }

                for (hash, output) in &scanned_block.wallet_outputs {
                    // Extract memo information
                    let payment_info = output.payment_id();
                    let memo_bytes = payment_info.get_payment_id();
                    let memo_parsed = if memo_bytes.is_empty() {
                        None
                    } else {
                        Some(String::from_utf8_lossy(&memo_bytes).to_string())
                    };
                    let memo_hex = if memo_bytes.is_empty() {
                        None
                    } else {
                        Some(hex::encode(&memo_bytes))
                    };

                    let event = models::WalletEvent {
                        id: 0,
                        account_id: account_type_row.account_id(),
                        event_type: models::WalletEventType::OutputDetected {
                            hash: *hash,
                            block_height: scanned_block.height,
                            block_hash: scanned_block.block_hash.to_vec(),
                            memo_parsed: memo_parsed.clone(),
                            memo_hex: memo_hex.clone(),
                        },
                        description: format!(
                            "Detected output with amount {} at height {}",
                            output.value(),
                            scanned_block.height
                        ),
                    };
                    result.push(event.clone());
                    let (output_id, inserted_new_output) = db::insert_output(
                        &mut tx,
                        account_type_row.account_id(),
                        hash.to_vec().clone(),
                        output,
                        scanned_block.height,
                        scanned_block.block_hash.as_slice(),
                        scanned_block.mined_timestamp,
                        memo_parsed.clone(),
                        memo_hex.clone(),
                    )
                    .await
                    .map_err(ScanError::FatalSqlx)?;

                    if inserted_new_output {
                        db::insert_wallet_event(&mut tx, account_type_row.account_id(), &event)
                            .await
                            .map_err(ScanError::Fatal)?;

                        // parse balance changes.
                        let balance_changes = parse_balance_changes(
                            account_type_row.account_id(),
                            output_id,
                            scanned_block.mined_timestamp,
                            scanned_block.height,
                            output,
                        );
                        for change in balance_changes {
                            db::insert_balance_change(&mut tx, &change)
                                .await
                                .map_err(ScanError::FatalSqlx)?;
                        }
                    }
                }
                db::insert_scanned_tip_block(
                    &mut tx,
                    account_type_row.account_id(),
                    scanned_block.height as i64,
                    scanned_block.block_hash.as_slice(),
                )
                .await
                .map_err(ScanError::FatalSqlx)?;

                // Check for outputs that should be confirmed (6 block confirmations)
                let unconfirmed_outputs = db::get_unconfirmed_outputs(
                    &mut tx,
                    account_type_row.account_id(),
                    scanned_block.height,
                    6, // 6 block confirmations required
                )
                .await
                .map_err(ScanError::FatalSqlx)?;

                for (output_hash, original_height, memo_parsed, memo_hex) in unconfirmed_outputs {
                    let confirmation_event = models::WalletEvent {
                        id: 0,
                        account_id: account_type_row.account_id(),
                        event_type: models::WalletEventType::OutputConfirmed {
                            hash: output_hash.clone(),
                            block_height: original_height,
                            confirmation_height: scanned_block.height,
                            memo_parsed,
                            memo_hex,
                        },
                        description: format!(
                            "Output confirmed at height {} (originally at height {})",
                            scanned_block.height, original_height
                        ),
                    };

                    result.push(confirmation_event.clone());
                    db::insert_wallet_event(&mut tx, account_type_row.account_id(), &confirmation_event)
                        .await
                        .map_err(ScanError::Fatal)?;

                    // Mark the output as confirmed in the database
                    db::mark_output_confirmed(
                        &mut tx,
                        &output_hash,
                        scanned_block.height,
                        scanned_block.block_hash.as_slice(),
                    )
                    .await
                    .map_err(ScanError::FatalSqlx)?;

                    // println!(
                    //     "Output {:?} confirmed at height {} (originally at height {})",
                    //     hex::encode(&output_hash),
                    //     scanned_block.height,
                    //     original_height
                    // );
                }

                // Commit transaction for this block's DB changes. If commit fails, return error and
                // rollback will be implicit when tx is dropped.
                tx.commit().await.map_err(ScanError::FatalSqlx)?;
            }

            // println!("Batch took {:?}.", timer.elapsed());
            // println!("pruning old scanned tip blocks...");
            db::prune_scanned_tip_blocks(
                &mut conn,
                account_type_row.account_id(),
                scanned_blocks.last().map(|b| b.height).unwrap_or(0),
            )
            .await
            .map_err(ScanError::Fatal)?;

            // println!("Cleanup took {:?}.", timer.elapsed());
        }

        println!("Scan complete. in {:?}", total_timer.elapsed());
        println!(
            "Took on average {:?}ms per block.",
            total_timer.elapsed().as_millis() / total_scanned as u128
        );
    }
    Ok(result)
}

fn parse_balance_changes(
    account_id: i64,
    output_id: i64,
    chain_timestamp: u64,
    chain_height: u64,
    output: &WalletOutput,
) -> Vec<models::BalanceChange> {
    // Coinbases.
    if output.features().is_coinbase() {
        let effective_date = DateTime::<Utc>::from_timestamp(chain_timestamp as i64, 0)
            .unwrap()
            .naive_utc();
        let balance_change = models::BalanceChange {
            account_id,
            caused_by_output_id: Some(output_id),
            caused_by_input_id: None,
            description: "Coinbase output found in blockchain scan".to_string(),
            balance_credit: output.value().as_u64(),
            balance_debit: 0,
            effective_date,
            effective_height: chain_height,
            claimed_recipient_address: None,
            memo_hex: None,
            memo_parsed: None,
            claimed_sender_address: None,
            claimed_fee: None,
            claimed_amount: None,
        };
        return vec![balance_change];
    }

    let mut changes = vec![];
    let effective_date = DateTime::<Utc>::from_timestamp(chain_timestamp as i64, 0)
        .unwrap()
        .naive_utc();
    let payment_info = output.payment_id();
    let memo_bytes = payment_info.get_payment_id();
    let memo = String::from_utf8_lossy(&memo_bytes);
    let memo_hex = hex::encode(payment_info.get_payment_id());
    let claimed_recipient_address = payment_info.get_recipient_address().map(|s| s.to_base58());
    let claimed_sender_address = payment_info.get_sender_address().map(|s| s.to_base58());
    let claimed_fee = payment_info.get_fee().map(|v| v.0);
    let claimed_amount = payment_info.get_amount().map(|v| v.0);

    let balance_change = models::BalanceChange {
        account_id,
        caused_by_output_id: Some(output_id),
        caused_by_input_id: None,
        description: "Output found in blockchain scan".to_string(),
        balance_credit: output.value().as_u64(),
        balance_debit: 0,
        effective_date,
        effective_height: chain_height,
        claimed_recipient_address,
        claimed_sender_address,
        memo_parsed: Some(memo.to_string()),
        memo_hex: Some(memo_hex),
        claimed_fee,
        claimed_amount,
    };
    changes.push(balance_change);
    changes
}
