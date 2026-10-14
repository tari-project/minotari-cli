//! Block processing logic for wallet output detection and balance tracking.
//!
//! This module contains the [`BlockProcessor`] which is responsible for analyzing
//! scanned blocks to detect wallet-owned outputs, track spending via inputs,
//! manage confirmation status, and emit events for each processed block.

use chrono::{DateTime, Utc};
use log::{error, info};
use minotari_scanning::BlockScanResult;
use rusqlite::Connection;
use std::collections::HashMap;
use tari_common_types::payment_reference::generate_payment_reference;
use tari_common_types::types::{FixedHash, PrivateKey};
use tari_transaction_components::MicroMinotari;
use tari_transaction_components::transaction_components::WalletOutput;
use thiserror::Error;

use crate::scan::block_event_accumulator::BlockEventAccumulator;
use crate::{
    db::{self, WalletDbError, get_burn_proof_by_output_hash},
    log::{mask_amount, mask_string},
    models::{BalanceChange, OutputStatus, WalletEvent, WalletEventType},
    scan::events::{
        DetectedOutput, DisplayedTransactionsEvent, EventSender, NoopEventSender, ProcessingEvent, ScanStatusEvent,
        SpentInput,
    },
    transactions::displayed_transaction_processor::DisplayedTransactionProcessor,
    webhooks::{WebhookTriggerConfig, utils::trigger_webhook_with_balance},
};

/// Errors that can occur during block processing.
#[derive(Debug, Error)]
pub enum BlockProcessorError {
    /// A database operation failed.
    #[error("Database execution error: {0}")]
    DbError(#[from] WalletDbError),

    /// Failed to insert or process a wallet event.
    #[error("Failed to insert wallet event: {0}")]
    WalletEvent(#[from] anyhow::Error),
}

/// Processes scanned blocks and persists wallet data.
///
/// The `BlockProcessor` is the workhorse of the scanning system. For each block,
/// it performs the following operations:
///
/// 1. **Output Detection**: Identifies outputs owned by the wallet and persists them
/// 2. **Input Tracking**: Detects when wallet outputs are spent as inputs
/// 3. **Confirmation Tracking**: Updates output confirmation status as blocks mature
/// 4. **Balance Changes**: Records all balance-affecting events
/// 5. **Event Emission**: Sends real-time events for UI updates
///
/// # Type Parameter
///
/// The processor is generic over the event sender type, defaulting to [`NoopEventSender`]
/// for cases where real-time events are not needed.
///
/// # Example
///
/// ```rust,ignore
/// use crate::scan::{BlockProcessor, ChannelEventSender};
///
/// // Create with event sender
/// let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
/// let event_sender = ChannelEventSender::new(tx);
/// let mut processor = BlockProcessor::with_event_sender(
///     vec![(account_id, view_key)],
///     event_sender,
///     false,
///     6,
/// );
///
/// // Process a block
/// processor.process_block(&tx, &scanned_block, account_id)?;
///
/// // Retrieve generated wallet events
/// let events = processor.into_wallet_events();
/// ```
pub struct BlockProcessor<E: EventSender = NoopEventSender> {
    /// Mapping of wallet index to account ID for active scanning set.
    wallet_index_to_account_id: Vec<i64>,
    /// Account view keys indexed by account ID.
    account_view_keys: HashMap<i64, PrivateKey>,
    /// Accumulated wallet events for the current processing session.
    wallet_events: Vec<WalletEvent>,
    /// Event sender for real-time notifications.
    event_sender: E,
    /// Accumulator for the block currently being processed.
    current_block: Option<BlockEventAccumulator>,
    /// Current blockchain tip height (for confirmation calculations).
    current_tip_height: u64,
    /// Whether there are pending outbound transactions to match against.
    has_pending_outbound: bool,
    /// Required confirmations
    required_confirmations: u64,
    /// Webhook Configuration
    webhook_config: Option<WebhookTriggerConfig>,
    /// When true, uses OR IGNORE inserts for outputs/inputs/balance_changes
    /// to skip already-existing records during history backfill (Phase 3 of FastSync).
    backfill_mode: bool,
}

impl BlockProcessor<NoopEventSender> {
    /// Creates a new block processor without event streaming.
    ///
    /// Use this constructor when you don't need real-time event updates
    /// and will retrieve wallet events via [`into_wallet_events`](Self::into_wallet_events)
    /// after processing.
    ///
    /// # Arguments
    ///
    /// * `accounts` - Ordered list mapping scanner wallet index to (account_id, account_view_key)
    pub fn new(accounts: Vec<(i64, PrivateKey)>, required_confirmations: u64) -> Self {
        let (wallet_index_to_account_id, account_view_keys) = Self::build_account_mappings(accounts);
        Self {
            wallet_index_to_account_id,
            account_view_keys,
            wallet_events: Vec::new(),
            event_sender: NoopEventSender,
            current_block: None,
            current_tip_height: 0,
            has_pending_outbound: false,
            required_confirmations,
            webhook_config: None,
            backfill_mode: false,
        }
    }
}

impl<E: EventSender> BlockProcessor<E> {
    /// Creates a new block processor with a custom event sender.
    ///
    /// Use this constructor when you need real-time event streaming during
    /// block processing.
    ///
    /// # Arguments
    ///
    /// * `accounts` - Ordered list mapping scanner wallet index to (account_id, account_view_key)
    /// * `event_sender` - Implementation of [`EventSender`] for event notifications
    /// * `has_pending_outbound` - Whether there are pending outbound transactions
    ///   that should be matched against scanned inputs
    pub fn with_event_sender(
        accounts: Vec<(i64, PrivateKey)>,
        event_sender: E,
        has_pending_outbound: bool,
        required_confirmations: u64,
    ) -> Self {
        let (wallet_index_to_account_id, account_view_keys) = Self::build_account_mappings(accounts);
        Self {
            wallet_index_to_account_id,
            account_view_keys,
            wallet_events: Vec::new(),
            event_sender,
            current_block: None,
            current_tip_height: 0,
            has_pending_outbound,
            required_confirmations,
            webhook_config: None,
            backfill_mode: false,
        }
    }

    fn build_account_mappings(accounts: Vec<(i64, PrivateKey)>) -> (Vec<i64>, HashMap<i64, PrivateKey>) {
        let wallet_index_to_account_id = accounts.iter().map(|(account_id, _)| *account_id).collect();
        let account_view_keys = accounts.into_iter().collect();
        (wallet_index_to_account_id, account_view_keys)
    }

    fn account_view_key(&self, account_id: i64) -> Result<&PrivateKey, BlockProcessorError> {
        self.account_view_keys
            .get(&account_id)
            .ok_or_else(|| BlockProcessorError::WalletEvent(anyhow::anyhow!("Unknown account id: {}", account_id)))
    }

    /// Updates the pending outbound transaction flag.
    ///
    /// Set to `true` when there are pending outbound transactions that should
    /// be matched against scanned inputs to link them with blockchain data.
    pub fn set_has_pending_outbound(&mut self, value: bool) {
        self.has_pending_outbound = value;
    }

    /// Sets webhook config
    pub fn set_webhook_config(&mut self, config: Option<WebhookTriggerConfig>) {
        self.webhook_config = config;
    }

    /// Enables or disables backfill mode for history backfill (Phase 3 of FastSync).
    /// In backfill mode, outputs and inputs use OR IGNORE inserts to skip duplicates.
    pub fn set_backfill_mode(&mut self, value: bool) {
        self.backfill_mode = value;
    }

    /// Processes a single scanned block.
    ///
    /// This is the main entry point for block processing. It performs all
    /// processing steps in sequence:
    ///
    /// 1. Process detected outputs (wallet-owned UTXOs)
    /// 2. Process inputs (spent outputs)
    /// 3. Record the scanned block for reorg detection
    /// 4. Update confirmation status for maturing outputs
    /// 5. Save and emit displayed transactions
    /// 6. Emit a [`BlockProcessedEvent`]
    ///
    /// # Arguments
    ///
    /// * `tx` - Database transaction for atomic operations
    /// * `block` - The scanned block result from the blockchain scanner
    /// * `account_id` - Account ID to process this block for
    ///
    /// # Errors
    ///
    /// Returns [`BlockProcessorError`] if any database operation fails.
    pub fn process_block(
        &mut self,
        tx: &Connection,
        block: &BlockScanResult,
        account_id: i64,
    ) -> Result<(), BlockProcessorError> {
        let account_view_key = self.account_view_key(account_id)?.clone();
        self.current_tip_height = block.height;

        self.current_block = Some(BlockEventAccumulator::new(
            account_id,
            block.height,
            block.block_hash.to_vec(),
        ));

        self.process_outputs(tx, block, account_id, &account_view_key)?;
        self.process_inputs(tx, block, account_id)?;
        self.record_scanned_block(tx, block, account_id)?;
        self.process_confirmations(tx, block, account_id)?;

        if let Some(acc) = self.current_block.take() {
            if !acc.is_empty() {
                self.save_and_emit_displayed_transactions(tx, block.height, account_id, &account_view_key, &acc)?;
            }

            let block_event = acc.into_event();
            self.event_sender.send(ProcessingEvent::BlockProcessed(block_event));
        }

        Ok(())
    }

    fn save_and_emit_displayed_transactions(
        &self,
        tx: &Connection,
        block_height: u64,
        account_id: i64,
        account_view_key: &PrivateKey,
        accumulator: &BlockEventAccumulator,
    ) -> Result<(), BlockProcessorError> {
        if accumulator.is_empty() {
            return Ok(());
        }

        let processor = DisplayedTransactionProcessor::new(
            self.current_tip_height,
            self.required_confirmations,
            account_view_key.clone(),
        );
        let (mut updated_transactions, mut new_transactions) = processor
            .create_new_updated_display_transactions_for_height(accumulator, tx)
            .map_err(|e| {
                error!(
                    block_height = block_height,
                    error:% = e;
                    "Failed to process displayed transactions"
                );
                BlockProcessorError::WalletEvent(anyhow::anyhow!("Failed to process displayed transactions: {}", e))
            })?;
        for updated in &updated_transactions {
            db::update_displayed_transaction_mined(tx, updated)?;
        }
        for new in &new_transactions {
            db::insert_displayed_transaction(tx, new)?;
        }
        updated_transactions.append(&mut new_transactions);
        self.event_sender
            .send(ProcessingEvent::TransactionsReady(DisplayedTransactionsEvent {
                account_id,
                transactions: updated_transactions,
                block_height: Some(block_height),
                is_initial_sync: false,
            }));

        Ok(())
    }

    /// Emits a scan status event through the event sender.
    ///
    /// This is a convenience method for emitting status updates during
    /// processing operations.
    pub fn emit_status(&self, status: ScanStatusEvent) {
        self.event_sender.send(ProcessingEvent::ScanStatus(status));
    }

    /// Consumes the processor and returns all accumulated wallet events.
    ///
    /// Call this after processing all blocks to retrieve the generated
    /// wallet events for persistence or further processing.
    pub fn into_wallet_events(self) -> Vec<WalletEvent> {
        self.wallet_events
    }

    /// Drains and returns accumulated wallet events while keeping the processor alive.
    pub fn take_wallet_events(&mut self) -> Vec<WalletEvent> {
        std::mem::take(&mut self.wallet_events)
    }

    /// Processes all detected outputs in the block.
    ///
    /// For each output owned by the wallet:
    /// - Inserts the output into the database
    /// - Creates a wallet event for detection
    /// - Records the balance change (credit)
    /// - Adds to the block accumulator for event emission
    fn process_outputs(
        &mut self,
        tx: &Connection,
        block: &BlockScanResult,
        account_id: i64,
        account_view_key: &PrivateKey,
    ) -> Result<(), BlockProcessorError> {
        let mut generated_events: Vec<(i64, WalletEvent)> = Vec::new();

        for (hash, output, wallet_idx) in &block.wallet_outputs {
            let Some(mapped_account_id) = self.wallet_index_to_account_id.get(*wallet_idx).copied() else {
                continue;
            };

            if mapped_account_id != account_id {
                continue;
            }

            let memo = MemoInfo::from_output(output);

            // Compute the payment reference from block hash and output hash
            let payment_reference = generate_payment_reference(&block.block_hash, hash);

            let is_burn = output.is_burned();

            // A send's change output was written at send time as `UNCONFIRMED`.
            // Update that row in place rather than inserting a second one (which
            // the active unique index on `output_hash` forbids); the credit,
            // event and accumulator entry below are recorded as for a new output.
            let status_when_found = if self.backfill_mode {
                OutputStatus::SpentUnconfirmed
            } else {
                OutputStatus::Unspent
            };
            let change_output_id = db::confirm_unconfirmed_change_output(
                tx,
                account_id,
                hash,
                block.height,
                &block.block_hash,
                block.mined_timestamp,
                memo.parsed.clone(),
                memo.hex.clone(),
                payment_reference,
                status_when_found,
            )?;

            let output_id = if let Some(id) = change_output_id {
                id
            } else if self.backfill_mode {
                match db::insert_spent_output_if_not_exists(
                    tx,
                    account_id,
                    account_view_key,
                    hash.to_vec(),
                    output,
                    block.height,
                    &block.block_hash,
                    block.mined_timestamp,
                    memo.parsed.clone(),
                    memo.hex.clone(),
                    payment_reference,
                    is_burn,
                )? {
                    Some(id) => id,
                    None => continue, // Output already exists, skip events and balance changes
                }
            } else {
                db::insert_output(
                    tx,
                    account_id,
                    account_view_key,
                    hash.to_vec(),
                    output,
                    block.height,
                    &block.block_hash,
                    block.mined_timestamp,
                    memo.parsed.clone(),
                    memo.hex.clone(),
                    payment_reference,
                    is_burn,
                )?
            };

            info!(
                target: "audit",
                account_id = account_id,
                block_height = block.height,
                value = &*mask_amount(output.value()),
                is_coinbase = output.features().is_coinbase();
                "Detected wallet output"
            );

            let event = self.make_output_detected_event(account_id, *hash, block, &memo);
            self.wallet_events.push(event.clone());

            let event_id = db::insert_wallet_event(tx, account_id, &event)?;
            generated_events.push((event_id, event));

            let balance_change = self.record_output_balance_change(tx, account_id, output_id, block, output)?;

            if let Some(ref mut acc) = self.current_block {
                acc.add_credit_change(
                    balance_change,
                    DetectedOutput {
                        height: block.height,
                        mined_in_block_hash: block.block_hash,
                        output: output.clone(),
                    },
                );
            }
        }

        if let Some(config) = &self.webhook_config {
            for (event_id, event) in generated_events {
                trigger_webhook_with_balance(tx, account_id, event_id, &event, config)?;
            }
        }

        Ok(())
    }

    /// Creates a wallet event for a newly detected output.
    fn make_output_detected_event(
        &self,
        account_id: i64,
        hash: FixedHash,
        block: &BlockScanResult,
        memo: &MemoInfo,
    ) -> WalletEvent {
        WalletEvent {
            id: 0,
            account_id,
            event_type: WalletEventType::OutputDetected {
                hash,
                block_height: block.height,
                block_hash: block.block_hash.to_vec(),
                memo_parsed: memo.parsed.clone(),
                memo_hex: memo.hex.clone(),
            },
            description: format!("Detected output at height {}", block.height),
        }
    }

    /// Records a balance change for a detected output (credit).
    fn record_output_balance_change(
        &self,
        tx: &Connection,
        account_id: i64,
        output_id: i64,
        block: &BlockScanResult,
        output: &WalletOutput,
    ) -> Result<BalanceChange, BlockProcessorError> {
        let change = make_balance_change_for_output(account_id, output_id, block.mined_timestamp, block.height, output);

        if self.backfill_mode {
            db::insert_balance_change_if_not_exists(tx, &change)?;
        } else {
            db::insert_balance_change(tx, &change)?;
        }

        Ok(change)
    }

    /// Processes all inputs in the block to detect spent outputs.
    ///
    /// For each input that references a wallet-owned output:
    /// - Inserts the input record into the database
    /// - Records the balance change (debit)
    /// - Updates the output status to Spent
    /// - Adds to the block accumulator for event emission
    fn process_inputs(
        &mut self,
        tx: &Connection,
        block: &BlockScanResult,
        account_id: i64,
    ) -> Result<(), BlockProcessorError> {
        for input_hash in &block.inputs {
            let Some((output_id, tx_id, output)) = db::get_output_info_by_hash_for_account(tx, account_id, input_hash)?
            else {
                continue;
            };

            info!(
                target: "audit",
                account_id = account_id,
                tx_id:% = tx_id,
                block_height = block.height,
                value = &*mask_amount(output.value());
                "Detected spent input"
            );

            let input_id = db::insert_input(
                tx,
                account_id,
                output_id,
                block.height,
                block.block_hash.as_slice(),
                block.mined_timestamp,
            )?;

            let balance_change = self.record_input_balance_change(tx, account_id, input_id, output.value(), block)?;
            db::update_output_status(tx, output_id, OutputStatus::Spent)?;

            if let Some(ref mut acc) = self.current_block {
                acc.add_debit_change(
                    balance_change,
                    SpentInput {
                        mined_in_block: block.block_hash,
                        mined_in_block_height: block.height,
                        output_id,
                        output,
                    },
                );
            }
        }

        Ok(())
    }

    /// Records a balance change for a spent input (debit).
    fn record_input_balance_change(
        &self,
        tx: &Connection,
        account_id: i64,
        input_id: i64,
        value: MicroMinotari,
        block: &BlockScanResult,
    ) -> Result<BalanceChange, BlockProcessorError> {
        #[allow(clippy::cast_possible_wrap)]
        let effective_date = DateTime::<Utc>::from_timestamp(block.mined_timestamp as i64, 0)
            .unwrap_or_else(Utc::now)
            .naive_utc();

        let change = BalanceChange {
            account_id,
            caused_by_output_id: None,
            caused_by_input_id: Some(input_id),
            description: "Output spent as input".to_string(),
            balance_credit: 0.into(),
            balance_debit: value,
            effective_date,
            effective_height: block.height,
            claimed_recipient_address: None,
            claimed_sender_address: None,
            memo_hex: None,
            memo_parsed: None,
            claimed_fee: None,
            claimed_amount: None,
            is_reversal: false,
            reversal_of_balance_change_id: None,
            is_reversed: false,
        };

        if self.backfill_mode {
            db::insert_balance_change_if_not_exists(tx, &change)?;
        } else {
            db::insert_balance_change(tx, &change)?;
        }
        Ok(change)
    }

    /// Records the scanned block for reorg detection.
    ///
    /// Stores the block height and hash in the scanned_tip_blocks table,
    /// which is used to detect chain reorganizations.
    fn record_scanned_block(
        &self,
        tx: &Connection,
        block: &BlockScanResult,
        account_id: i64,
    ) -> Result<(), BlockProcessorError> {
        #[allow(clippy::cast_possible_wrap)]
        db::insert_scanned_tip_block(tx, account_id, block.height as i64, block.block_hash.as_slice())?;
        Ok(())
    }

    /// Processes confirmation status updates for maturing outputs.
    ///
    /// Finds outputs that have reached the required confirmation depth
    /// and updates their status to confirmed.
    fn process_confirmations(
        &mut self,
        tx: &Connection,
        block: &BlockScanResult,
        account_id: i64,
    ) -> Result<(), BlockProcessorError> {
        let mut generated_events: Vec<(i64, WalletEvent)> = Vec::new();

        let unconfirmed_outputs =
            db::get_unconfirmed_outputs(tx, account_id, block.height, self.required_confirmations)?;

        for unconfirmed_output in unconfirmed_outputs {
            info!(
                target: "audit",
                account_id = account_id,
                output_hash = &*mask_string(&hex::encode(unconfirmed_output.output_hash)),
                original_height = unconfirmed_output.mined_in_block_height,
                confirmed_at = block.height;
                "Output confirmed"
            );

            let event = self.make_confirmation_event(
                account_id,
                &unconfirmed_output.output_hash,
                unconfirmed_output.mined_in_block_height as u64,
                block.height,
                unconfirmed_output.memo_parsed,
                unconfirmed_output.memo_hex,
            );

            self.wallet_events.push(event.clone());
            let event_id = db::insert_wallet_event(tx, account_id, &event)?;
            generated_events.push((event_id, event));

            db::mark_output_confirmed(
                tx,
                &unconfirmed_output.output_hash,
                block.height,
                block.block_hash.as_slice(),
            )?;

            // If this is a burn output, check if we have a matching pending burn proof
            // and log that the daemon should pick it up for burn output proof fetching.
            if unconfirmed_output.is_burn != 0 {
                match get_burn_proof_by_output_hash(tx, &unconfirmed_output.output_hash) {
                    Ok(Some(_proof)) => {
                        info!(
                            target: "audit",
                            output_hash = &*mask_string(&hex::encode(unconfirmed_output.output_hash));
                            "Burn output confirmed — daemon will fetch burn output proof"
                        );
                    },
                    Ok(None) => {
                        // Burn output detected but not created by this wallet instance; nothing to do.
                    },
                    Err(e) => {
                        error!(
                            output_hash = &*mask_string(&hex::encode(unconfirmed_output.output_hash)),
                            error:% = e;
                            "Failed to look up burn proof for confirmed burn output"
                        );
                    },
                }
            }
        }

        if let Some(config) = &self.webhook_config {
            for (event_id, event) in generated_events {
                trigger_webhook_with_balance(tx, account_id, event_id, &event, config)?;
            }
        }

        Ok(())
    }

    /// Creates a wallet event for an output reaching confirmation depth.
    fn make_confirmation_event(
        &self,
        account_id: i64,
        output_hash: &FixedHash,
        original_height: u64,
        confirmation_height: u64,
        memo_parsed: Option<String>,
        memo_hex: Option<String>,
    ) -> WalletEvent {
        WalletEvent {
            id: 0,
            account_id,
            event_type: WalletEventType::OutputConfirmed {
                hash: *output_hash,
                block_height: original_height,
                confirmation_height,
                memo_parsed,
                memo_hex,
            },
            description: format!(
                "Output confirmed at height {} (originally at {})",
                confirmation_height, original_height
            ),
        }
    }
}

/// Extracted memo information from an output's payment ID.
pub struct MemoInfo {
    /// Human-readable interpretation of the memo (UTF-8 lossy).
    pub parsed: Option<String>,
    /// Hexadecimal encoding of the raw memo bytes.
    pub hex: Option<String>,
}

impl MemoInfo {
    /// Extracts memo information from a wallet output's payment ID.
    pub fn from_output(output: &WalletOutput) -> Self {
        let payment_info = output.payment_id();
        let memo_bytes = payment_info.get_payment_id();

        if memo_bytes.is_empty() {
            Self {
                parsed: None,
                hex: None,
            }
        } else {
            Self {
                parsed: Some(String::from_utf8_lossy(&memo_bytes).to_string()),
                hex: Some(hex::encode(&memo_bytes)),
            }
        }
    }
}

/// Creates a balance change record for a detected output.
///
/// Handles both regular outputs and coinbase outputs, extracting payment
/// information including recipient/sender addresses, fees, and amounts
/// from the output's payment ID.
fn make_balance_change_for_output(
    account_id: i64,
    output_id: i64,
    timestamp: u64,
    height: u64,
    output: &WalletOutput,
) -> BalanceChange {
    #[allow(clippy::cast_possible_wrap)]
    let effective_date = DateTime::<Utc>::from_timestamp(timestamp as i64, 0)
        .unwrap_or_else(Utc::now)
        .naive_utc();

    let payment_info = output.payment_id();
    let memo_bytes = payment_info.get_payment_id();

    if output.features().is_coinbase() {
        return BalanceChange {
            account_id,
            caused_by_output_id: Some(output_id),
            caused_by_input_id: None,
            description: "Coinbase output found in blockchain scan".to_string(),
            balance_credit: output.value(),
            balance_debit: 0.into(),
            effective_date,
            effective_height: height,
            claimed_recipient_address: None,
            claimed_sender_address: None,
            memo_parsed: Some(String::from_utf8_lossy(&memo_bytes).to_string()),
            memo_hex: Some(hex::encode(&memo_bytes)),
            claimed_fee: payment_info.get_fee(),
            claimed_amount: payment_info.get_amount(),
            is_reversal: false,
            reversal_of_balance_change_id: None,
            is_reversed: false,
        };
    }

    BalanceChange {
        account_id,
        caused_by_output_id: Some(output_id),
        caused_by_input_id: None,
        description: "Output found in blockchain scan".to_string(),
        balance_credit: output.value(),
        balance_debit: 0.into(),
        effective_date,
        effective_height: height,
        claimed_recipient_address: payment_info.get_recipient_address(),
        claimed_sender_address: payment_info.get_sender_address(),
        memo_parsed: Some(String::from_utf8_lossy(&memo_bytes).to_string()),
        memo_hex: Some(hex::encode(&memo_bytes)),
        claimed_fee: payment_info.get_fee(),
        claimed_amount: payment_info.get_amount(),
        is_reversal: false,
        reversal_of_balance_change_id: None,
        is_reversed: false,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::cast_possible_wrap)]

    use super::*;
    use crate::db::{SqlitePool, create_account, get_account_by_name, init_db};
    use rusqlite::named_params;
    use tari_common_types::seeds::cipher_seed::CipherSeed;
    use tari_common_types::types::{ComAndPubSignature, CompressedPublicKey};
    use tari_script::{ExecutionStack, TariScript};
    use tari_transaction_components::key_manager::wallet_types::{SeedWordsWallet, WalletType};
    use tari_transaction_components::key_manager::{KeyManager, TariKeyId, TransactionKeyManagerInterface};
    use tari_transaction_components::transaction_components::{
        EncryptedData, MemoField, OutputFeatures, TransactionOutputVersion, covenants::Covenant,
    };
    use tempfile::TempDir;

    struct TestWallet {
        pool: SqlitePool,
        account_id: i64,
        view_key: PrivateKey,
        _dir: TempDir,
    }

    fn test_wallet() -> TestWallet {
        let dir = tempfile::tempdir().expect("temp dir");
        let pool = init_db(dir.path().join("processor.db")).expect("init db");
        let conn = pool.get().expect("conn");
        let wallet = WalletType::SeedWords(SeedWordsWallet::construct_new(CipherSeed::random()).unwrap());
        create_account(&conn, "default", &wallet, "password").unwrap();
        let account_id = get_account_by_name(&conn, "default").unwrap().unwrap().id;
        let view_key = KeyManager::new(wallet).unwrap().get_private_view_key();
        TestWallet {
            pool,
            account_id,
            view_key,
            _dir: dir,
        }
    }

    fn processor(wallet: &TestWallet, has_pending_outbound: bool) -> BlockProcessor {
        let mut processor = BlockProcessor::new(vec![(wallet.account_id, wallet.view_key.clone())], 3);
        processor.set_has_pending_outbound(has_pending_outbound);
        processor
    }

    fn hash(byte: u8) -> FixedHash {
        FixedHash::from([byte; 32])
    }

    fn test_wallet_output(value: u64, hash_byte: u8) -> WalletOutput {
        WalletOutput::new_from_parts(
            TransactionOutputVersion::default(),
            MicroMinotari::from(value),
            TariKeyId::default(),
            OutputFeatures::default(),
            TariScript::default(),
            ExecutionStack::default(),
            TariKeyId::default(),
            CompressedPublicKey::default(),
            ComAndPubSignature::default(),
            0,
            Covenant::default(),
            EncryptedData::default(),
            MicroMinotari::from(0),
            None,
            MemoField::new_empty(),
            hash(hash_byte),
            Default::default(),
        )
    }

    fn block(height: u64, wallet_outputs: Vec<WalletOutput>, inputs: Vec<FixedHash>) -> BlockScanResult {
        BlockScanResult {
            height,
            #[allow(clippy::cast_possible_truncation)]
            block_hash: FixedHash::from([200u8.wrapping_add(height as u8); 32]),
            wallet_outputs: wallet_outputs.into_iter().map(|o| (o.output_hash(), o, 0)).collect(),
            inputs,
            mined_timestamp: 1_700_000_000,
        }
    }

    fn output_row(conn: &Connection, output_hash: &FixedHash) -> (i64, String, i64, Vec<u8>, Option<String>) {
        conn.query_row(
            "SELECT id, status, mined_in_block_height, mined_in_block_hash, payment_reference
             FROM outputs WHERE output_hash = :hash AND deleted_at IS NULL",
            named_params! { ":hash": output_hash.as_slice() },
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        )
        .expect("output row")
    }

    #[test]
    fn the_change_row_is_updated_in_place_when_the_scan_finds_it() {
        let wallet = test_wallet();
        let conn = wallet.pool.get().expect("conn");
        let change = test_wallet_output(5_000, 7);
        let change_id =
            db::insert_unconfirmed_change_output(&conn, wallet.account_id, &wallet.view_key, "request-1", &change)
                .expect("insert change");
        let (_, status, height, _, payref) = output_row(&conn, &change.output_hash());
        assert_eq!(status, "UNCONFIRMED");
        assert_eq!(height, 0, "sentinel height until mined");
        assert!(payref.is_none());

        let mined_block = block(50, vec![change.clone()], Vec::new());
        processor(&wallet, false)
            .process_block(&conn, &mined_block, wallet.account_id)
            .expect("process block");

        let (id, status, height, block_hash, payref) = output_row(&conn, &change.output_hash());
        assert_eq!(id, change_id, "the same row is updated, not a second one inserted");
        assert_eq!(status, "UNSPENT");
        assert_eq!(height, 50);
        assert_eq!(block_hash, mined_block.block_hash.to_vec());
        assert!(payref.is_some(), "the payment reference is set when mined");

        let credit_id = db::get_balance_change_id_by_output(&conn, change_id).expect("query");
        assert!(credit_id.is_some(), "the credit balance change is recorded");
    }
}
