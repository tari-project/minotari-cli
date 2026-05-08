//! Top-level orchestrator for the console-wallet -> minotari-cli migration.
//!
//! ```text
//!   ConsoleWalletReader  +-> derive cipher seed from password
//!                        +-> reads outputs, completed_transactions, scanned_blocks
//!                                       v
//!   output_converter     reconstructs WalletOutput per row
//!                                       v
//!   tx_converter         builds DisplayedTransaction per completed_transactions row
//!                                       v
//!   migrator (this file) writes accounts / outputs / balance_changes / inputs /
//!                        displayed_transactions / scanned_tip_blocks
//! ```
//!
//! All inserts happen inside a single SQLite transaction. If any step fails
//! the whole thing rolls back, leaving the destination wallet untouched.

use std::path::PathBuf;

use anyhow::{Context, anyhow};
use chrono::{DateTime, NaiveDateTime, Utc};
use log::info;
use rusqlite::{Connection, named_params};
use tari_common_types::{seeds::cipher_seed::CipherSeed, types::FixedHash};
use tari_transaction_components::MicroMinotari;
use tari_transaction_components::key_manager::wallet_types::{SeedWordsWallet, WalletType};

use crate::db::{self, init_db};
use crate::models::{BalanceChange, OutputStatus};

use super::console_db::{ConsoleCompletedTxRow, ConsoleScannedTip, ConsoleWalletReader};
use super::output_converter::{ConvertedOutput, LegacyOutputStatus, convert_output};
use super::tx_converter::{convert_transaction, decode_output_hashes};

/// Inputs to a migration run.
#[derive(Clone, Debug)]
pub struct MigrationOptions {
    /// Path to the legacy console wallet's SQLite file.
    pub source_db_path: PathBuf,
    /// Passphrase that unlocks the legacy wallet.
    pub source_passphrase: String,
    /// Path to the new minotari-cli SQLite file. Created if missing; the
    /// account is added alongside any existing accounts.
    pub destination_db_path: PathBuf,
    /// Passphrase used to encrypt the new account's wallet blob.
    pub destination_passphrase: String,
    /// Friendly name to give the new account.
    pub account_name: String,
}

/// Summary returned to the caller for display / testing.
#[derive(Debug, Default)]
pub struct MigrationReport {
    pub account_name: String,
    pub outputs_migrated: usize,
    pub outputs_skipped: usize,
    pub unspent_outputs_count: usize,
    pub spent_outputs_count: usize,
    pub balance_credit: MicroMinotari,
    pub balance_debit: MicroMinotari,
    pub displayed_transactions_migrated: usize,
    pub scan_tip_height: Option<u64>,
}

impl MigrationReport {
    pub fn net_balance(&self) -> MicroMinotari {
        self.balance_credit.saturating_sub(self.balance_debit)
    }
}

/// Run the migration end-to-end. Returns a report on success.
///
/// Steps:
/// 1. Open the source DB and decrypt the cipher seed using the source passphrase.
/// 2. Open / create the destination DB and run its migrations.
/// 3. Open a write transaction on the destination DB.
/// 4. Create the destination account (encrypted with the destination passphrase).
/// 5. Migrate each output.
/// 6. Migrate each completed transaction into displayed_transactions.
/// 7. Set the scan tip so the new wallet does not re-scan ground the console
///    wallet already covered.
/// 8. Commit, or roll back on any error along the way.
pub fn run_migration(options: MigrationOptions) -> Result<MigrationReport, anyhow::Error> {
    if options.source_db_path == options.destination_db_path {
        return Err(anyhow!(
            "Source and destination database paths cannot be the same"
        ));
    }

    info!(
        target: "audit",
        source = options.source_db_path.display().to_string().as_str(),
        dest = options.destination_db_path.display().to_string().as_str(),
        account = options.account_name.as_str();
        "Starting console-wallet -> minotari-cli migration"
    );

    // 1. Open source DB and authenticate.
    let (reader, cipher_seed) = ConsoleWalletReader::open(&options.source_db_path, &options.source_passphrase)
        .context("Failed to open and authenticate the source console wallet")?;

    // 2. Read all source data eagerly. The dataset is bounded by the user's
    //    own UTXO set / transaction history and easily fits in memory; this
    //    keeps the destination write transaction short-lived.
    let outputs = reader.read_outputs().context("Failed to read source outputs")?;
    let transactions = reader
        .read_completed_transactions()
        .context("Failed to read source completed_transactions")?;
    let scan_tip = reader
        .read_latest_scanned_block()
        .context("Failed to read source scanned_blocks")?;
    drop(reader); // close source DB before we touch the destination

    // 3. Initialise destination DB and run migrations.
    let pool = init_db(options.destination_db_path.clone())
        .map_err(|e| anyhow!("Failed to initialise destination database: {e}"))?;
    let mut conn = pool.get().context("Failed to get destination DB connection")?;

    // 4. Single transaction for the whole migration. We use IMMEDIATE so the
    //    write lock is acquired up-front, making the duplicate-name check
    //    below atomic vs any concurrent writer.
    let tx = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .context("Failed to start migration transaction")?;

    // Reject duplicate account names; checked inside the transaction so a
    // concurrent writer can't race in between the check and create_account.
    if db::get_account_by_name(&tx, &options.account_name)
        .map_err(|e| anyhow!("Lookup of existing account failed: {e}"))?
        .is_some()
    {
        return Err(anyhow!(
            "Destination already has an account named '{}'; refusing to overwrite",
            options.account_name
        ));
    }

    let report = migrate_in_transaction(&tx, &cipher_seed, &options, &outputs, &transactions, &scan_tip)?;
    tx.commit().context("Failed to commit migration transaction")?;

    info!(
        target: "audit",
        outputs = report.outputs_migrated,
        skipped = report.outputs_skipped,
        unspent = report.unspent_outputs_count,
        displayed = report.displayed_transactions_migrated,
        balance = report.net_balance().as_u64();
        "Migration committed"
    );

    Ok(report)
}

fn migrate_in_transaction(
    tx: &rusqlite::Transaction<'_>,
    cipher_seed: &CipherSeed,
    options: &MigrationOptions,
    outputs: &[super::console_db::ConsoleOutputRow],
    transactions: &[ConsoleCompletedTxRow],
    scan_tip: &Option<ConsoleScannedTip>,
) -> Result<MigrationReport, anyhow::Error> {
    let mut report = MigrationReport {
        account_name: options.account_name.clone(),
        ..Default::default()
    };

    // 4a. Create the new account from the recovered seed.
    let seed_wallet = SeedWordsWallet::construct_new(cipher_seed.clone())
        .map_err(|e| anyhow!("Failed to construct SeedWordsWallet from migrated seed: {e}"))?;
    let wallet_type = WalletType::SeedWords(seed_wallet);
    db::create_account(tx, &options.account_name, &wallet_type, &options.destination_passphrase)
        .map_err(|e| anyhow!("Failed to create destination account: {e}"))?;

    let account_id: i64 = tx
        .query_row(
            "SELECT id FROM accounts WHERE friendly_name = ?1",
            [&options.account_name],
            |r| r.get(0),
        )
        .context("Failed to look up newly-created account id")?;

    // Map of console-wallet `outputs.received_in_tx_id` -> destination
    // `outputs.id` so we can wire up `inputs` rows later for spent outputs.
    let mut output_id_by_console_received_tx_id: std::collections::HashMap<u64, (i64, MicroMinotari, FixedHash, u64, FixedHash, NaiveDateTime)> =
        std::collections::HashMap::new();
    let mut output_id_by_hash: std::collections::HashMap<FixedHash, i64> = std::collections::HashMap::new();

    // 5. Outputs.
    for raw in outputs {
        match convert_output(raw)? {
            None => report.outputs_skipped += 1,
            Some(converted) => {
                insert_converted_output(tx, account_id, &converted, &mut report)?;
                let inserted_id = tx.last_insert_rowid();
                output_id_by_hash.insert(converted.output_hash, inserted_id);
                if let Some(rx_id) = converted.received_in_tx_id {
                    output_id_by_console_received_tx_id.insert(
                        rx_id,
                        (
                            inserted_id,
                            converted.value,
                            converted.output_hash,
                            converted.mined_height,
                            converted.mined_block_hash,
                            converted.mined_timestamp,
                        ),
                    );
                }
                report.outputs_migrated += 1;
                if converted.legacy_status.is_unspent() {
                    report.unspent_outputs_count += 1;
                    insert_credit_balance_change(tx, account_id, &converted, inserted_id)?;
                    report.balance_credit = report.balance_credit.saturating_add(converted.value);
                } else if converted.legacy_status.is_spent() {
                    report.spent_outputs_count += 1;
                    // For spent outputs we need both a credit (the receive) and
                    // a debit (the spend). The credit + spend pair keeps the
                    // balance arithmetic consistent and lets the user see a
                    // historical trail.
                    insert_credit_balance_change(tx, account_id, &converted, inserted_id)?;
                    report.balance_credit = report.balance_credit.saturating_add(converted.value);
                    insert_input_for_spent_output(tx, account_id, &converted, inserted_id)?;
                    let input_id = tx.last_insert_rowid();
                    insert_debit_balance_change(tx, account_id, &converted, input_id)?;
                    report.balance_debit = report.balance_debit.saturating_add(converted.value);
                }
            }
        }
    }

    // 6. Completed transactions -> displayed_transactions. Preserves the
    //    console wallet's random tx_id values as the user-facing ID.
    for raw_tx in transactions {
        let sent_hashes = decode_output_hashes(raw_tx.sent_output_hashes.as_ref());
        let converted = convert_transaction(raw_tx, account_id, sent_hashes)?;
        // The displayed_transactions PRIMARY KEY is text; we use the legacy
        // u64 stringified to preserve the exact value the user is used to.
        // Hard-fail rather than skip: a partial transaction history is worse
        // than aborting and letting the user re-attempt.
        db::insert_displayed_transaction(tx, &converted.display).with_context(|| {
            format!(
                "Failed to migrate displayed transaction with legacy tx_id {}",
                raw_tx.tx_id
            )
        })?;
        report.displayed_transactions_migrated += 1;
    }

    // 7. Scan tip. Avoids re-scanning the chain from genesis on the next
    //    `tari scan` invocation.
    if let Some(tip) = scan_tip {
        let height = i64::try_from(tip.height).unwrap_or(i64::MAX);
        tx.execute(
            "INSERT INTO scanned_tip_blocks (account_id, height, hash) VALUES (:account_id, :height, :hash)",
            named_params! {
                ":account_id": account_id,
                ":height": height,
                ":hash": tip.header_hash,
            },
        )
        .context("Failed to insert scanned_tip_blocks marker")?;
        report.scan_tip_height = Some(tip.height);
    }

    Ok(report)
}

fn insert_converted_output(
    tx: &Connection,
    account_id: i64,
    converted: &ConvertedOutput,
    _report: &mut MigrationReport,
) -> Result<(), anyhow::Error> {
    let output_json = serde_json::to_string(&converted.wallet_output)
        .context("Failed to serialise migrated WalletOutput as JSON")?;
    let value_i64 = converted.value.as_u64() as i64;
    let height_i64 = i64::try_from(converted.mined_height).unwrap_or(i64::MAX);
    let mined_dt = DateTime::<Utc>::from_naive_utc_and_offset(converted.mined_timestamp, Utc);
    // Preserve the console wallet's tx_id for received outputs so the user can
    // still cross-reference legacy IDs after migration; for sent outputs (which
    // never get a console "received_in_tx_id") fall back to a deterministic id.
    let tx_id = match converted.received_in_tx_id {
        Some(id) => id as i64,
        None => {
            // i64 wrap is fine because the column is just an opaque identifier
            // alongside `output_hash`; the latter is what the rest of the
            // wallet keys off.
            #[allow(clippy::cast_possible_wrap)]
            let v = converted.output_hash.as_slice();
            // Use the first 8 bytes of the output hash as a stable derived id;
            // collisions are astronomically unlikely.
            i64::from_le_bytes(<[u8; 8]>::try_from(&v[..8]).unwrap_or([0; 8]))
        }
    };

    let status_label = match converted.legacy_status {
        LegacyOutputStatus::Spent | LegacyOutputStatus::SpentMinedUnconfirmed => OutputStatus::Spent.to_string(),
        LegacyOutputStatus::EncumberedToBeSpent | LegacyOutputStatus::ShortTermEncumberedToBeSpent => {
            OutputStatus::Locked.to_string()
        }
        _ => OutputStatus::Unspent.to_string(),
    };

    tx.execute(
        r#"
        INSERT INTO outputs (
            account_id, tx_id, output_hash, mined_in_block_height,
            mined_in_block_hash, value, mined_timestamp, wallet_output_json,
            status
        ) VALUES (
            :account_id, :tx_id, :hash, :height, :block_hash, :value,
            :ts, :json, :status
        )
        "#,
        named_params! {
            ":account_id": account_id,
            ":tx_id": tx_id,
            ":hash": converted.output_hash.as_slice(),
            ":height": height_i64,
            ":block_hash": converted.mined_block_hash.as_slice(),
            ":value": value_i64,
            ":ts": mined_dt,
            ":json": output_json,
            ":status": status_label,
        },
    )
    .context("Failed to insert migrated output")?;

    Ok(())
}

fn insert_credit_balance_change(
    tx: &Connection,
    account_id: i64,
    converted: &ConvertedOutput,
    output_id: i64,
) -> Result<(), anyhow::Error> {
    let change = BalanceChange {
        account_id,
        caused_by_output_id: Some(output_id),
        caused_by_input_id: None,
        description: "migrated from console wallet".to_string(),
        balance_credit: converted.value,
        balance_debit: MicroMinotari::from(0),
        effective_date: converted.mined_timestamp,
        effective_height: converted.mined_height,
        claimed_recipient_address: None,
        claimed_sender_address: None,
        memo_parsed: None,
        memo_hex: None,
        claimed_fee: None,
        claimed_amount: Some(converted.value),
        is_reversal: false,
        reversal_of_balance_change_id: None,
        is_reversed: false,
    };
    db::insert_balance_change(tx, &change).map_err(|e| anyhow!("Failed to insert credit balance_change: {e}"))?;
    Ok(())
}

fn insert_input_for_spent_output(
    tx: &Connection,
    account_id: i64,
    converted: &ConvertedOutput,
    output_id: i64,
) -> Result<(), anyhow::Error> {
    // We use the *spent* event's block info if we had it, otherwise we fall
    // back to the original mined block. The console wallet doesn't track the
    // exact spent-block separately on the output row, so this is the best we
    // have without re-scanning.
    let mined_dt = DateTime::<Utc>::from_naive_utc_and_offset(converted.mined_timestamp, Utc);
    tx.execute(
        r#"
        INSERT INTO inputs (
            account_id, output_id, mined_in_block_height,
            mined_in_block_hash, mined_timestamp
        ) VALUES (
            :account_id, :output_id, :height, :block_hash, :ts
        )
        "#,
        named_params! {
            ":account_id": account_id,
            ":output_id": output_id,
            ":height": i64::try_from(converted.mined_height).unwrap_or(i64::MAX),
            ":block_hash": converted.mined_block_hash.as_slice(),
            ":ts": mined_dt,
        },
    )
    .context("Failed to insert input row for spent output")?;
    Ok(())
}

fn insert_debit_balance_change(
    tx: &Connection,
    account_id: i64,
    converted: &ConvertedOutput,
    input_id: i64,
) -> Result<(), anyhow::Error> {
    let change = BalanceChange {
        account_id,
        caused_by_output_id: None,
        caused_by_input_id: Some(input_id),
        description: "migrated spent (debit) from console wallet".to_string(),
        balance_credit: MicroMinotari::from(0),
        balance_debit: converted.value,
        effective_date: converted.mined_timestamp,
        effective_height: converted.mined_height,
        claimed_recipient_address: None,
        claimed_sender_address: None,
        memo_parsed: None,
        memo_hex: None,
        claimed_fee: None,
        claimed_amount: Some(converted.value),
        is_reversal: false,
        reversal_of_balance_change_id: None,
        is_reversed: false,
    };
    db::insert_balance_change(tx, &change).map_err(|e| anyhow!("Failed to insert debit balance_change: {e}"))?;
    Ok(())
}
