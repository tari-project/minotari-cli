//! Block processing logic for wallet output detection and balance tracking.
//!
//! This module contains the [`BlockProcessor`] which is responsible for analyzing
//! scanned blocks to detect wallet-owned outputs, track spending via inputs,
//! manage confirmation status, and emit events for each processed block.

use chrono::{DateTime, Utc};
use log::{error, info, warn};
use minotari_scanning::BlockScanResult;
use rusqlite::Connection;
use std::collections::{HashMap, HashSet};
use tari_common_types::payment_reference::generate_payment_reference;
use tari_common_types::types::{FixedHash, PrivateKey};
use tari_transaction_components::MicroMinotari;
use tari_transaction_components::transaction_components::{Transaction, WalletOutput};
use thiserror::Error;

use crate::scan::block_event_accumulator::BlockEventAccumulator;
use crate::{
    db::{self, WalletDbError, get_burn_proof_by_output_hash},
    log::{mask_amount, mask_string},
    models::{BalanceChange, OutputStatus, WalletEvent, WalletEventType},
    scan::events::{
        DetectedOutput, DisplayedTransactionsEvent, EventSender, NoopEventSender, ProcessingEvent, ScanStatusEvent,
        SpentInput, TransactionsUpdatedEvent,
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
    /// What the block being processed showed of the wallet's own sends. Only
    /// collected while `has_pending_outbound` is set. See [`Self::match_sends`].
    send_block: Option<SendBlock>,
}

/// The parts of one block needed to recognise the wallet's own sends in it.
///
/// A block with many outputs or inputs arrives as several [`BlockScanResult`]s
/// that share its hash, so these are collected over all of them and the sends
/// are matched once the whole block is in.
struct SendBlock {
    account_id: i64,
    height: u64,
    block_hash: FixedHash,
    /// Every input hash in the block.
    inputs: HashSet<FixedHash>,
    /// Hashes of the block's outputs that belong to this account.
    wallet_outputs: HashSet<FixedHash>,
    /// `locked_by_request_id` of every wallet output the block spends.
    request_ids: Vec<String>,
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
            send_block: None,
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
            send_block: None,
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
    /// When `false`, spent inputs are not looked up as the wallet's own sends,
    /// which keeps long syncs fast.
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

        // The previous block is complete once a result with another hash arrives.
        let starts_new_block = match &self.send_block {
            Some(send_block) => send_block.block_hash != block.block_hash || send_block.account_id != account_id,
            None => false,
        };
        if starts_new_block {
            self.match_sends(tx)?;
        }

        self.current_tip_height = block.height;

        self.current_block = Some(BlockEventAccumulator::new(
            account_id,
            block.height,
            block.block_hash.to_vec(),
        ));

        self.process_outputs(tx, block, account_id, &account_view_key)?;
        let request_ids = self.process_inputs(tx, block, account_id)?;
        if self.has_pending_outbound {
            self.collect_send_block(block, account_id, request_ids);
        }
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
    ///
    /// While `has_pending_outbound` is set, also returns the
    /// `locked_by_request_id` of every spent output that had one: the sends
    /// that may have landed in this block.
    fn process_inputs(
        &mut self,
        tx: &Connection,
        block: &BlockScanResult,
        account_id: i64,
    ) -> Result<Vec<String>, BlockProcessorError> {
        let mut request_ids = Vec::new();
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

            if self.has_pending_outbound
                && let Some(request_id) = db::get_output_lock_request_id(tx, output_id)?
            {
                request_ids.push(request_id);
            }

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

        Ok(request_ids)
    }

    /// Adds this block result's inputs, wallet outputs and spent requests to
    /// [`Self::send_block`].
    fn collect_send_block(&mut self, block: &BlockScanResult, account_id: i64, request_ids: Vec<String>) {
        let send_block = self.send_block.get_or_insert_with(|| SendBlock {
            account_id,
            height: block.height,
            block_hash: block.block_hash,
            inputs: HashSet::new(),
            wallet_outputs: HashSet::new(),
            request_ids: Vec::new(),
        });
        send_block.inputs.extend(block.inputs.iter().copied());
        for (hash, _output, wallet_idx) in &block.wallet_outputs {
            if self.wallet_index_to_account_id.get(*wallet_idx).copied() == Some(account_id) {
                send_block.wallet_outputs.insert(*hash);
            }
        }
        send_block.request_ids.extend(request_ids);
    }

    /// Matches the wallet's own sends against the last block collected by
    /// [`Self::collect_send_block`]. Call after the last block of a batch;
    /// earlier blocks are matched when the next one starts.
    ///
    /// A send whose locked input was spent in the block is looked up (only
    /// `Completed` or `Broadcast` sends), once per block. It counts as mined
    /// in this block when every one of its inputs is spent in the block and
    /// its change output appears in the block. A send without a change row
    /// (made before change was required, or assembled outside this crate) is
    /// matched on its inputs alone.
    ///
    /// A mined send is marked `MinedUnconfirmed`. Otherwise its inputs were
    /// spent by a different transaction: it is marked `Rejected` (recording the
    /// block so a reorg can revert it), its change `REJECTED` and its displayed
    /// transaction rejected. Its inputs are not unlocked: they really are spent.
    ///
    /// This relies on the wallet seeing the whole block. A mismatch is logged at
    /// `warn` on the audit target.
    pub fn match_sends(&mut self, tx: &Connection) -> Result<(), BlockProcessorError> {
        let Some(send_block) = self.send_block.take() else {
            return Ok(());
        };

        let mut generated_events: Vec<(i64, WalletEvent)> = Vec::new();
        let mut rejected_displayed_transactions = Vec::new();
        let mut seen_request_ids = HashSet::new();

        for request_id in &send_block.request_ids {
            if !seen_request_ids.insert(request_id.as_str()) {
                continue;
            }
            let Some(completed) =
                db::get_unmined_completed_transaction_by_pending_tx_id(tx, send_block.account_id, request_id)?
            else {
                continue;
            };

            // A row that cannot be decoded must not fail the block: the batch
            // would roll back and the same block be retried for ever.
            let transaction: Transaction = match serde_json::from_slice(&completed.serialized_transaction) {
                Ok(transaction) => transaction,
                Err(e) => {
                    warn!(
                        target: "audit",
                        id = completed.id.to_string().as_str(),
                        error:% = e;
                        "Cannot decode stored transaction; not matching it against this block"
                    );
                    continue;
                },
            };

            let mut all_inputs_in_block = true;
            for input in transaction.body().inputs() {
                if !send_block.inputs.contains(&input.output_hash()) {
                    all_inputs_in_block = false;
                }
            }

            let change_in_block = match db::get_change_output_hash_for_request(tx, request_id)? {
                Some(change_hash) => send_block.wallet_outputs.contains(&change_hash),
                // Legacy send: no change row to look for, so the inputs decide.
                None => true,
            };

            #[allow(clippy::cast_possible_wrap)]
            let block_height = send_block.height as i64;

            if all_inputs_in_block && change_in_block {
                info!(
                    target: "audit",
                    id = completed.id.to_string().as_str(),
                    height = send_block.height;
                    "Transaction mined (unconfirmed)"
                );
                db::mark_completed_transaction_as_mined_unconfirmed(
                    tx,
                    completed.id,
                    block_height,
                    send_block.block_hash.as_slice(),
                )?;

                let event = WalletEvent {
                    id: 0,
                    account_id: send_block.account_id,
                    event_type: WalletEventType::TransactionUnconfirmed {
                        tx_id: completed.id,
                        mined_height: send_block.height,
                        confirmations: 0,
                    },
                    description: format!("Transaction {} mined at height {}", completed.id, send_block.height),
                };
                let event_id = db::insert_wallet_event(tx, send_block.account_id, &event)?;
                self.wallet_events.push(event.clone());
                generated_events.push((event_id, event));
                continue;
            }

            let reason = "inputs spent by a different transaction".to_string();
            warn!(
                target: "audit",
                id = completed.id.to_string().as_str(),
                height = send_block.height,
                all_inputs_in_block = all_inputs_in_block,
                change_in_block = change_in_block;
                "Transaction inputs spent by a different transaction; marking it rejected"
            );
            db::mark_completed_transaction_as_rejected_in_block(
                tx,
                completed.id,
                &reason,
                block_height,
                send_block.block_hash.as_slice(),
            )?;
            db::mark_change_output_rejected(tx, request_id)?;
            if let Some(displayed) = db::mark_displayed_transaction_rejected(tx, completed.id)? {
                rejected_displayed_transactions.push(displayed);
            }

            let event = WalletEvent {
                id: 0,
                account_id: send_block.account_id,
                event_type: WalletEventType::TransactionRejected {
                    tx_id: completed.id,
                    reason,
                },
                description: format!(
                    "Transaction {} rejected: its inputs were spent by a different transaction at height {}",
                    completed.id, send_block.height
                ),
            };
            let event_id = db::insert_wallet_event(tx, send_block.account_id, &event)?;
            self.wallet_events.push(event.clone());
            generated_events.push((event_id, event));
        }

        if !rejected_displayed_transactions.is_empty() {
            self.event_sender
                .send(ProcessingEvent::TransactionsUpdated(TransactionsUpdatedEvent {
                    account_id: send_block.account_id,
                    updated_transactions: rejected_displayed_transactions,
                }));
        }

        if let Some(config) = &self.webhook_config {
            for (event_id, event) in generated_events {
                trigger_webhook_with_balance(tx, send_block.account_id, event_id, &event, config)?;
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
    #![allow(clippy::indexing_slicing)]

    use super::*;
    use crate::db::{SqlitePool, create_account, get_account_by_name, init_db};
    use crate::transactions::idempotency::{IdempotencyBinding, IdempotencyOperation, RequestFingerprint};
    use crate::transactions::{
        BlockchainInfo, DisplayedTransaction, TransactionDetails, TransactionDirection, TransactionDisplayStatus,
        TransactionSource,
    };
    use rusqlite::named_params;
    use tari_common_types::seeds::cipher_seed::CipherSeed;
    use tari_common_types::transaction::TxId;
    use tari_common_types::types::{ComAndPubSignature, CompressedPublicKey};
    use tari_script::{ExecutionStack, TariScript};
    use tari_transaction_components::key_manager::wallet_types::{SeedWordsWallet, WalletType};
    use tari_transaction_components::key_manager::{KeyManager, TariKeyId, TransactionKeyManagerInterface};
    use tari_transaction_components::transaction_components::{
        EncryptedData, MemoField, OutputFeatures, TransactionInput, TransactionOutputVersion, covenants::Covenant,
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

    /// A broadcast send recorded the way `TransactionManager` records one.
    struct TestSend {
        request_id: String,
        tx_id: TxId,
        input_hashes: Vec<FixedHash>,
        change: Option<WalletOutput>,
    }

    /// Records a `Broadcast` send spending new wallet outputs `input_bytes`
    /// (locked to the send), with a pending displayed transaction and, when
    /// `change_byte` is given, an `UNCONFIRMED` change row.
    fn record_send(
        wallet: &TestWallet,
        conn: &Connection,
        id: u64,
        input_bytes: &[u8],
        change_byte: Option<u8>,
    ) -> TestSend {
        let binding = IdempotencyBinding::new(
            Some(format!("send-{id}")),
            IdempotencyOperation::SendTransaction,
            RequestFingerprint::new(IdempotencyOperation::SendTransaction),
        );
        let request_id = db::create_pending_transaction(
            conn,
            &format!("send-{id}"),
            &binding,
            wallet.account_id,
            true,
            MicroMinotari(0),
            MicroMinotari(0),
            MicroMinotari(0),
            Utc::now() + chrono::Duration::hours(1),
        )
        .expect("pending transaction");

        let mut input_hashes = Vec::new();
        let mut inputs = Vec::new();
        for byte in input_bytes {
            let output = test_wallet_output(10_000, *byte);
            let output_id = db::insert_output(
                conn,
                wallet.account_id,
                &wallet.view_key,
                output.output_hash().to_vec(),
                &output,
                10,
                &hash(10),
                1_700_000_000,
                None,
                None,
                FixedHash::zero(),
                false,
            )
            .expect("input output");
            db::lock_output(conn, output_id, &request_id, Utc::now()).expect("lock input");
            input_hashes.push(output.output_hash());
            inputs.push(TransactionInput::new_with_output_hash(
                output.output_hash(),
                ExecutionStack::default(),
                ComAndPubSignature::default(),
            ));
        }

        let transaction = Transaction::new(
            inputs,
            Vec::new(),
            Vec::new(),
            PrivateKey::default(),
            PrivateKey::default(),
        );
        let tx_id = TxId::from(id);
        db::create_completed_transaction(
            conn,
            wallet.account_id,
            &request_id,
            &[],
            &serde_json::to_vec(&transaction).expect("serialize"),
            None,
            tx_id,
        )
        .expect("completed transaction");
        db::mark_completed_transaction_as_broadcasted(conn, tx_id, 1).expect("broadcast");

        let change = change_byte.map(|byte| test_wallet_output(3_000, byte));
        if let Some(change) = &change {
            db::insert_unconfirmed_change_output(conn, wallet.account_id, &wallet.view_key, &request_id, change)
                .expect("change row");
        }

        db::insert_displayed_transaction(conn, &pending_displayed_send(wallet.account_id, tx_id)).expect("displayed");

        TestSend {
            request_id,
            tx_id,
            input_hashes,
            change,
        }
    }

    fn pending_displayed_send(account_id: i64, tx_id: TxId) -> DisplayedTransaction {
        DisplayedTransaction {
            id: tx_id,
            direction: TransactionDirection::Outgoing,
            source: TransactionSource::OneSided,
            status: TransactionDisplayStatus::Pending,
            amount: MicroMinotari::from(1_000),
            message: None,
            counterparty: None,
            blockchain: BlockchainInfo {
                block_height: 0,
                timestamp: Utc::now().naive_utc(),
                confirmations: 0,
                block_hash: FixedHash::zero(),
            },
            fee: None,
            details: TransactionDetails {
                account_id,
                total_credit: MicroMinotari::from(0),
                total_debit: MicroMinotari::from(1_000),
                inputs: Vec::new(),
                outputs: Vec::new(),
                output_type: None,
                coinbase_extra: None,
                memo_hex: None,
                sent_output_hashes: Vec::new(),
                sent_payrefs: Vec::new(),
            },
            lock_height: 0,
        }
    }

    fn completed(conn: &Connection, tx_id: TxId) -> db::CompletedTransaction {
        db::get_completed_transaction_by_id(conn, tx_id)
            .expect("query")
            .expect("completed transaction exists")
    }

    fn change_row(conn: &Connection, send: &TestSend) -> (i64, String, i64, Option<String>) {
        conn.query_row(
            "SELECT id, status, mined_in_block_height, payment_reference
             FROM outputs WHERE created_by_request_id = :req AND deleted_at IS NULL",
            named_params! { ":req": send.request_id },
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("change row")
    }

    fn input_statuses(conn: &Connection, send: &TestSend) -> Vec<String> {
        let mut statuses = Vec::new();
        for input_hash in &send.input_hashes {
            statuses.push(output_row(conn, input_hash).1);
        }
        statuses
    }

    fn stored_event_types(conn: &Connection, account_id: i64) -> Vec<String> {
        db::get_events_by_account_id(conn, account_id, 100, 0)
            .expect("events")
            .into_iter()
            .map(|e| e.event_type)
            .collect()
    }

    /// Processes `blocks` as one batch, the way `ScanDbHandler` does.
    fn process(processor: &mut BlockProcessor, conn: &Connection, account_id: i64, blocks: &[BlockScanResult]) {
        for block in blocks {
            processor.process_block(conn, block, account_id).expect("process block");
        }
        processor.match_sends(conn).expect("match sends");
    }

    #[test]
    fn a_send_that_cannot_be_decoded_is_skipped_without_failing_the_block() {
        let wallet = test_wallet();
        let conn = wallet.pool.get().expect("conn");
        let send = record_send(&wallet, &conn, 1, &[1, 2], Some(3));
        conn.execute("UPDATE completed_transactions SET serialized_transaction = x'00ff'", [])
            .expect("corrupt");

        let mined_block = block(50, vec![send.change.clone().unwrap()], send.input_hashes.clone());
        let mut processor = processor(&wallet, true);
        process(
            &mut processor,
            &conn,
            wallet.account_id,
            std::slice::from_ref(&mined_block),
        );

        assert_eq!(
            completed(&conn, send.tx_id).status,
            db::CompletedTransactionStatus::Broadcast
        );
        // The rest of the block was still processed.
        assert_eq!(change_row(&conn, &send).1, "UNSPENT");
    }

    #[test]
    fn a_send_seen_in_a_block_is_marked_mined_unconfirmed() {
        let wallet = test_wallet();
        let conn = wallet.pool.get().expect("conn");
        let send = record_send(&wallet, &conn, 1, &[1, 2], Some(3));

        let mined_block = block(50, vec![send.change.clone().unwrap()], send.input_hashes.clone());
        let mut processor = processor(&wallet, true);
        process(
            &mut processor,
            &conn,
            wallet.account_id,
            std::slice::from_ref(&mined_block),
        );

        let completed = completed(&conn, send.tx_id);
        assert_eq!(completed.status, db::CompletedTransactionStatus::MinedUnconfirmed);
        assert_eq!(completed.mined_height, Some(50));
        assert_eq!(completed.mined_block_hash, Some(mined_block.block_hash.to_vec()));

        let events = processor.take_wallet_events();
        assert!(events.iter().any(|e| matches!(
            e.event_type,
            WalletEventType::TransactionUnconfirmed { tx_id, mined_height: 50, confirmations: 0 } if tx_id == send.tx_id
        )));
        assert!(stored_event_types(&conn, wallet.account_id).contains(&"TransactionUnconfirmed".to_string()));
        assert_eq!(change_row(&conn, &send).1, "UNSPENT");
    }

    #[test]
    fn a_send_split_over_several_results_of_one_block_is_still_matched() {
        // A large block arrives as several results sharing its hash; the send's
        // inputs and change can be in different ones.
        let wallet = test_wallet();
        let conn = wallet.pool.get().expect("conn");
        let send = record_send(&wallet, &conn, 1, &[1, 2], Some(3));

        let first = block(50, vec![send.change.clone().unwrap()], vec![send.input_hashes[0]]);
        let second = block(50, Vec::new(), vec![send.input_hashes[1]]);
        let mut processor = processor(&wallet, true);
        process(&mut processor, &conn, wallet.account_id, &[first, second]);

        assert_eq!(
            completed(&conn, send.tx_id).status,
            db::CompletedTransactionStatus::MinedUnconfirmed
        );
    }

    #[test]
    fn a_send_whose_inputs_were_spent_by_another_transaction_is_rejected_without_unlocking() {
        let wallet = test_wallet();
        let conn = wallet.pool.get().expect("conn");
        let send = record_send(&wallet, &conn, 1, &[1, 2], Some(3));

        // Only one of the inputs is spent, and the change is not in the block.
        let other_block = block(50, Vec::new(), vec![send.input_hashes[0]]);
        let mut processor = processor(&wallet, true);
        process(
            &mut processor,
            &conn,
            wallet.account_id,
            std::slice::from_ref(&other_block),
        );

        let completed = completed(&conn, send.tx_id);
        assert_eq!(completed.status, db::CompletedTransactionStatus::Rejected);
        assert_eq!(
            completed.last_rejected_reason.as_deref(),
            Some("inputs spent by a different transaction")
        );
        assert_eq!(
            completed.mined_height,
            Some(50),
            "the block is recorded for a reorg revert"
        );
        assert_eq!(completed.mined_block_hash, Some(other_block.block_hash.to_vec()));

        assert_eq!(change_row(&conn, &send).1, "REJECTED");

        let displayed = db::get_displayed_transaction_by_id(&conn, &send.tx_id.to_string())
            .expect("query")
            .expect("displayed");
        assert_eq!(displayed.status, TransactionDisplayStatus::Rejected);

        // Nothing is unlocked: the spent input is spent, the other stays locked.
        assert_eq!(
            input_statuses(&conn, &send),
            vec!["SPENT".to_string(), "LOCKED".to_string()]
        );

        let events = processor.take_wallet_events();
        assert!(events.iter().any(|e| matches!(
            &e.event_type,
            WalletEventType::TransactionRejected { tx_id, reason }
                if *tx_id == send.tx_id && reason == "inputs spent by a different transaction"
        )));
        assert!(stored_event_types(&conn, wallet.account_id).contains(&"TransactionRejected".to_string()));
    }

    #[test]
    fn a_legacy_send_without_a_change_row_is_matched_on_its_inputs_alone() {
        let wallet = test_wallet();
        let conn = wallet.pool.get().expect("conn");
        let send = record_send(&wallet, &conn, 1, &[1, 2], None);

        let mined_block = block(50, Vec::new(), send.input_hashes.clone());
        let mut processor = processor(&wallet, true);
        process(
            &mut processor,
            &conn,
            wallet.account_id,
            std::slice::from_ref(&mined_block),
        );

        assert_eq!(
            completed(&conn, send.tx_id).status,
            db::CompletedTransactionStatus::MinedUnconfirmed
        );
    }

    #[test]
    fn without_pending_outbound_sends_are_not_looked_up() {
        let wallet = test_wallet();
        let conn = wallet.pool.get().expect("conn");
        let send = record_send(&wallet, &conn, 1, &[1, 2], Some(3));

        let mined_block = block(50, vec![send.change.clone().unwrap()], send.input_hashes.clone());
        let mut processor = processor(&wallet, false);
        process(
            &mut processor,
            &conn,
            wallet.account_id,
            std::slice::from_ref(&mined_block),
        );

        assert_eq!(
            completed(&conn, send.tx_id).status,
            db::CompletedTransactionStatus::Broadcast
        );
        let events = processor.take_wallet_events();
        assert!(
            !events
                .iter()
                .any(|e| matches!(e.event_type, WalletEventType::TransactionUnconfirmed { .. }))
        );
    }

    #[test]
    fn a_reorg_reverts_a_mined_send_and_keeps_its_change_as_unconfirmed() {
        let wallet = test_wallet();
        let mut conn = wallet.pool.get().expect("conn");
        let send = record_send(&wallet, &conn, 1, &[1, 2], Some(3));

        let mined_block = block(50, vec![send.change.clone().unwrap()], send.input_hashes.clone());
        let mut mine_processor = processor(&wallet, true);
        process(
            &mut mine_processor,
            &conn,
            wallet.account_id,
            std::slice::from_ref(&mined_block),
        );
        let (change_id, ..) = change_row(&conn, &send);

        let tx = conn.transaction().expect("tx");
        let info = crate::scan::reorg::rollback_from_height(&tx, wallet.account_id, 50, None).expect("rollback");
        tx.commit().expect("commit");

        let completed = completed(&conn, send.tx_id);
        assert_eq!(completed.status, db::CompletedTransactionStatus::Completed);
        assert_eq!(completed.mined_height, None);

        let (id, status, height, payref) = change_row(&conn, &send);
        assert_eq!(id, change_id, "the change row is kept, not soft-deleted");
        assert_eq!(status, "UNCONFIRMED");
        assert_eq!(height, 0);
        assert!(payref.is_none());
        assert!(
            db::get_balance_change_id_by_output(&conn, change_id)
                .expect("query")
                .is_none(),
            "the change's credit is reversed"
        );

        let change_hash = send.change.as_ref().unwrap().output_hash();
        assert!(info.invalidated_output_hashes.contains(&change_hash));
        assert!(stored_event_types(&conn, wallet.account_id).contains(&"OutputRolledBack".to_string()));

        // Re-mined after the reorg, it is found again by the same row.
        let remined = block(51, vec![send.change.clone().unwrap()], send.input_hashes.clone());
        db::mark_completed_transaction_as_broadcasted(&conn, send.tx_id, 1).expect("rebroadcast");
        let mut remine_processor = processor(&wallet, true);
        process(
            &mut remine_processor,
            &conn,
            wallet.account_id,
            std::slice::from_ref(&remined),
        );
        assert_eq!(change_row(&conn, &send).0, change_id);
        assert_eq!(change_row(&conn, &send).1, "UNSPENT");
    }

    #[test]
    fn a_reorg_reverts_a_scan_rejected_send_and_its_change() {
        let wallet = test_wallet();
        let mut conn = wallet.pool.get().expect("conn");
        let send = record_send(&wallet, &conn, 1, &[1, 2], Some(3));

        let other_block = block(50, Vec::new(), vec![send.input_hashes[0]]);
        let mut processor = processor(&wallet, true);
        process(
            &mut processor,
            &conn,
            wallet.account_id,
            std::slice::from_ref(&other_block),
        );
        assert_eq!(
            completed(&conn, send.tx_id).status,
            db::CompletedTransactionStatus::Rejected
        );

        let tx = conn.transaction().expect("tx");
        crate::scan::reorg::rollback_from_height(&tx, wallet.account_id, 50, None).expect("rollback");
        tx.commit().expect("commit");

        let completed = completed(&conn, send.tx_id);
        assert_eq!(completed.status, db::CompletedTransactionStatus::Completed);
        assert_eq!(completed.mined_height, None);
        assert_eq!(completed.last_rejected_reason, None);
        assert_eq!(completed.broadcast_attempts, 0);
        assert_eq!(change_row(&conn, &send).1, "UNCONFIRMED");
    }
}
