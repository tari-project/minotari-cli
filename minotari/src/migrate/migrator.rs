// Copyright 2026 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use anyhow::{Context, anyhow};
use rusqlite::{Connection, named_params};
use tari_common_types::{seeds::cipher_seed::CipherSeed, types::PrivateKey};
use tari_transaction_components::{
    MicroMinotari,
    key_manager::{KeyManager, wallet_types::SeedWordsWallet, wallet_types::WalletType},
};

use crate::{
    db::{self, SqlitePool},
    models::BalanceChange,
};

use super::{
    console_db::{ConsoleCompletedTx, ConsoleDb, ConsoleOutput, ConsoleSyncTip},
    output_converter::{
        ConvertedOutput, LegacyKeyBlocker, assign_destination_tx_ids, convert_output, detect_legacy_key_blocker,
    },
    tx_converter::{ConvertedTransaction, convert_transaction},
};

#[derive(Debug, Clone, Default)]
pub struct MigrationReport {
    pub transactions_found: usize,
    pub transactions_imported: usize,
    pub outputs_found: usize,
    pub outputs_imported: usize,
    pub outputs_blocked_legacy_key: usize,
    pub blocked_output_hashes: Vec<String>,
    pub outputs_skipped_legacy_key: usize,
    pub skipped_output_hashes: Vec<String>,
    pub source_balance_utari: u64,
    pub imported_balance_utari: u64,
    pub balance_match: bool,
    pub scan_tip_height: Option<u64>,
    pub dry_run: bool,
    pub partial_import: bool,
    pub account_name: String,
}

impl MigrationReport {
    pub fn print(&self) {
        println!("Migration Report");
        println!("================");
        println!("Mode:              {}", if self.dry_run { "DRY RUN" } else { "LIVE" });
        println!("Partial import:    {}", if self.partial_import { "YES" } else { "NO" });
        println!(
            "Transactions:      {} found | {} imported",
            self.transactions_found, self.transactions_imported
        );
        println!(
            "Outputs:           {} found | {} imported",
            self.outputs_found, self.outputs_imported
        );
        println!("Legacy blockers:   {}", self.outputs_blocked_legacy_key);
        println!("Legacy skipped:    {}", self.outputs_skipped_legacy_key);
        println!("Source balance:    {} µXTM", self.source_balance_utari);
        println!("Imported balance:  {} µXTM", self.imported_balance_utari);
        println!("Balance match:     {}", if self.balance_match { "YES" } else { "NO" });
        match self.scan_tip_height {
            Some(height) => println!("Scan tip:          block {}", height),
            None => println!("Scan tip:          none"),
        }

        if !self.blocked_output_hashes.is_empty() {
            println!("Blocked output hashes:");
            for hash in &self.blocked_output_hashes {
                println!("  - {}", hash);
            }
        }

        if !self.skipped_output_hashes.is_empty() {
            println!("Skipped output hashes:");
            for hash in &self.skipped_output_hashes {
                println!("  - {}", hash);
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct MigrationOptions<'a> {
    pub account_name: &'a str,
    pub password: &'a str,
    pub dry_run: bool,
    pub allow_partial_import: bool,
    pub account_view_key: &'a PrivateKey,
}

pub fn run_migration(
    console_db: &ConsoleDb,
    cipher_seed: &CipherSeed,
    dest_pool: &SqlitePool,
    options: MigrationOptions<'_>,
) -> Result<MigrationReport, anyhow::Error> {
    let MigrationOptions {
        account_name,
        password,
        dry_run,
        allow_partial_import,
        account_view_key,
    } = options;

    let source_transactions = console_db
        .read_completed_transactions()
        .context("Failed to read legacy completed_transactions")?;
    let source_outputs = console_db
        .read_unspent_outputs()
        .context("Failed to read legacy outputs")?;
    let explicit_sync_tip = console_db
        .read_sync_tip()
        .context("Failed to inspect legacy sync tip")?;
    let fallback_sync_tip = console_db
        .fallback_max_mined_height()
        .context("Failed to compute fallback legacy sync tip")?;

    let blockers: Vec<LegacyKeyBlocker> = source_outputs.iter().filter_map(detect_legacy_key_blocker).collect();
    let blocked_output_hashes: Vec<String> = blockers.iter().map(|b| b.output_hash_hex.clone()).collect();

    if !blockers.is_empty() && !dry_run && !allow_partial_import {
        let first = &blockers[0];
        return Err(anyhow!(
            "Legacy key format detected for output {} ({}). Please run the latest minotari_console_wallet binary once to upgrade your wallet's key format, then retry.",
            first.output_hash_hex,
            first.field_name
        ));
    }

    let mut converted_outputs = Vec::new();
    let mut skipped_output_hashes = Vec::new();
    for output in &source_outputs {
        match detect_legacy_key_blocker(output) {
            Some(blocker) if allow_partial_import && !dry_run => {
                skipped_output_hashes.push(blocker.output_hash_hex);
            },
            Some(_) => {},
            None => converted_outputs.push(convert_output(output)?),
        }
    }
    assign_destination_tx_ids(&mut converted_outputs, account_view_key);

    let converted_transactions = source_transactions
        .iter()
        .map(|tx| convert_transaction(tx, 0))
        .collect::<Result<Vec<_>, _>>()?;

    let source_balance_utari: u64 = converted_outputs
        .iter()
        .map(|output| output.wallet_output.value().as_u64())
        .sum();
    let resolved_sync_tip = resolve_sync_tip(&explicit_sync_tip, &converted_outputs, fallback_sync_tip);

    let mut report = MigrationReport {
        transactions_found: source_transactions.len(),
        transactions_imported: converted_transactions.len(),
        outputs_found: source_outputs.len(),
        outputs_imported: converted_outputs.len(),
        outputs_blocked_legacy_key: blocked_output_hashes.len(),
        blocked_output_hashes,
        outputs_skipped_legacy_key: skipped_output_hashes.len(),
        skipped_output_hashes,
        source_balance_utari,
        imported_balance_utari: if dry_run { source_balance_utari } else { 0 },
        balance_match: dry_run,
        scan_tip_height: resolved_sync_tip.as_ref().map(|tip| tip.height),
        dry_run,
        partial_import: allow_partial_import,
        account_name: account_name.to_string(),
    };

    if dry_run {
        report.print();
        return Ok(report);
    }

    let mut conn = dest_pool.get().context("Failed to get destination DB connection")?;

    if db::get_account_by_name(&conn, account_name)?.is_some() {
        return Err(anyhow!("Destination account '{}' already exists", account_name));
    }

    let tx = conn
        .transaction()
        .context("Failed to start destination migration transaction")?;
    let account_id = create_destination_account(&tx, cipher_seed, account_name, password)?;

    let mut output_row_ids_by_source_tx_id = std::collections::HashMap::new();
    for output in &converted_outputs {
        let output_row_id = insert_output_row(&tx, account_id, output)?;
        if let Some(source_tx_id) = output.original_received_in_tx_id {
            output_row_ids_by_source_tx_id
                .entry(source_tx_id)
                .or_insert(output_row_id);
        }
        insert_output_credit_balance_change(&tx, account_id, output_row_id, output)?;
    }

    for source_tx in &source_transactions {
        let mut converted = convert_transaction(source_tx, account_id)?;
        if let Some(output_row_id) = output_row_ids_by_source_tx_id.get(&source_tx.tx_id).copied()
            && matches!(
                converted.displayed.direction,
                crate::transactions::TransactionDirection::Incoming
            )
        {
            converted.metadata_balance_change.caused_by_output_id = Some(output_row_id);
        }

        db::insert_balance_change(&tx, &converted.metadata_balance_change)?;
        db::insert_displayed_transaction(&tx, &converted.displayed)?;
    }

    if let Some(sync_tip) = &resolved_sync_tip {
        db::insert_scanned_tip_block(&tx, account_id, sync_tip.height as i64, &sync_tip.block_hash)?;
    }

    let imported_balance_utari = db::get_total_unspent_balance(&tx, account_id)?;
    report.imported_balance_utari = imported_balance_utari;
    report.balance_match = imported_balance_utari == report.source_balance_utari;

    if !report.balance_match {
        return Err(anyhow!(
            "Imported balance {} does not match source balance {}",
            imported_balance_utari,
            report.source_balance_utari
        ));
    }

    tx.commit().context("Failed to commit migrated wallet data")?;
    report.print();
    Ok(report)
}

fn create_destination_account(
    conn: &Connection,
    cipher_seed: &CipherSeed,
    account_name: &str,
    password: &str,
) -> Result<i64, anyhow::Error> {
    let seed_wallet = SeedWordsWallet::construct_new(cipher_seed.clone())
        .map_err(|e| anyhow!("Failed to reconstruct source seed wallet: {}", e))?;
    let wallet = WalletType::SeedWords(seed_wallet);
    let _key_manager =
        KeyManager::new(wallet.clone()).map_err(|e| anyhow!("Failed to build destination key manager: {}", e))?;
    db::create_account(conn, account_name, &wallet, password)?;

    let account = db::get_account_by_name(conn, account_name)?
        .ok_or_else(|| anyhow!("Failed to fetch newly created account '{}'", account_name))?;
    Ok(account.id)
}

fn insert_output_row(conn: &Connection, account_id: i64, output: &ConvertedOutput) -> Result<i64, anyhow::Error> {
    let output_json = serde_json::to_string(&output.wallet_output)
        .with_context(|| format!("Failed to serialize WalletOutput {}", hex::encode(output.output_hash)))?;

    conn.execute(
        r#"
        INSERT INTO outputs (
            account_id,
            tx_id,
            output_hash,
            mined_in_block_hash,
            mined_in_block_height,
            value,
            wallet_output_json,
            mined_timestamp,
            confirmed_height,
            confirmed_hash,
            status
        ) VALUES (
            :account_id,
            :tx_id,
            :output_hash,
            :block_hash,
            :block_height,
            :value,
            :wallet_output_json,
            :mined_timestamp,
            :confirmed_height,
            :confirmed_hash,
            :status
        )
        "#,
        named_params! {
            ":account_id": account_id,
            ":tx_id": output.destination_tx_id.as_i64_wrapped(),
            ":output_hash": output.output_hash.as_slice(),
            ":block_hash": output.mined_block_hash.as_slice(),
            ":block_height": output.mined_height as i64,
            ":value": output.wallet_output.value().as_u64() as i64,
            ":wallet_output_json": output_json,
            ":mined_timestamp": output.mined_timestamp,
            ":confirmed_height": output.mined_height as i64,
            ":confirmed_hash": output.mined_block_hash.as_slice(),
            ":status": "UNSPENT",
        },
    )?;

    Ok(conn.last_insert_rowid())
}

fn insert_output_credit_balance_change(
    conn: &Connection,
    account_id: i64,
    output_row_id: i64,
    output: &ConvertedOutput,
) -> Result<(), anyhow::Error> {
    let raw = output.wallet_output.payment_id().to_bytes();
    let memo_hex = if raw.iter().all(|b| *b == 0) {
        None
    } else {
        Some(hex::encode(raw))
    };
    let balance_change = BalanceChange {
        account_id,
        caused_by_output_id: Some(output_row_id),
        caused_by_input_id: None,
        description: format!("Migrated console wallet output {}", hex::encode(output.output_hash)),
        balance_credit: output.wallet_output.value(),
        balance_debit: MicroMinotari::from(0),
        effective_date: output.mined_timestamp,
        effective_height: output.mined_height,
        claimed_recipient_address: None,
        claimed_sender_address: None,
        memo_parsed: None,
        memo_hex,
        claimed_fee: None,
        claimed_amount: Some(output.wallet_output.value()),
        is_reversal: false,
        reversal_of_balance_change_id: None,
        is_reversed: false,
    };

    db::insert_balance_change(conn, &balance_change)?;
    Ok(())
}

fn resolve_sync_tip(
    explicit_sync_tip: &Option<ConsoleSyncTip>,
    converted_outputs: &[ConvertedOutput],
    fallback_max_height: Option<u64>,
) -> Option<ConsoleSyncTip> {
    if let Some(sync_tip) = explicit_sync_tip {
        return Some(sync_tip.clone());
    }

    if let Some(max_height) = fallback_max_height
        && let Some(output) = converted_outputs
            .iter()
            .find(|output| output.mined_height == max_height)
    {
        return Some(ConsoleSyncTip {
            height: max_height,
            block_hash: output.mined_block_hash.to_vec(),
        });
    }

    converted_outputs
        .iter()
        .max_by_key(|output| output.mined_height)
        .map(|output| ConsoleSyncTip {
            height: output.mined_height,
            block_hash: output.mined_block_hash.to_vec(),
        })
}

#[allow(dead_code)]
fn _linkable_source_tx_id(output: &ConvertedOutput) -> Option<i64> {
    output.original_received_in_tx_id
}

#[allow(dead_code)]
fn _converted_transaction_count(transactions: &[ConvertedTransaction]) -> usize {
    transactions.len()
}

#[allow(dead_code)]
fn _source_output_count(outputs: &[ConsoleOutput]) -> usize {
    outputs.len()
}

#[allow(dead_code)]
fn _source_transaction_count(transactions: &[ConsoleCompletedTx]) -> usize {
    transactions.len()
}
