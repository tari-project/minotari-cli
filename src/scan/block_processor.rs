//! Block processing logic for wallet output detection and balance tracking.
//!
//! This module contains the [`BlockProcessor`] which is responsible for analyzing
//! scanned blocks to detect wallet-owned outputs, track spending via inputs,
//! manage confirmation status, and emit events for each processed block.

use chrono::{DateTime, Utc};
use lightweight_wallet_libs::BlockScanResult;
use rusqlite::Connection;
use tari_common_types::types::FixedHash;
use tari_transaction_components::transaction_components::WalletOutput;
use thiserror::Error;

use crate::{
    db::{self, WalletDbError},
    models::{BalanceChange, OutputStatus, WalletEvent, WalletEventType},
    scan::events::{
        BalanceChangeSummary, BlockProcessedEvent, ConfirmedOutput, DetectedOutput, DisplayedTransactionsEvent,
        EventSender, NoopEventSender, ProcessingEvent, ScanStatusEvent, SpentInput,
    },
    transactions::{
        DisplayedTransaction, TransactionDirection, TransactionDisplayStatus,
        displayed_transaction_processor::{DisplayedTransactionProcessor, ProcessingContext},
        monitor::REQUIRED_CONFIRMATIONS,
    },
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
///     account_id,
///     view_key.to_vec(),
///     event_sender,
///     false,
/// );
///
/// // Process a block
/// processor.process_block(&mut tx, &scanned_block).await?;
///
/// // Retrieve generated wallet events
/// let events = processor.into_wallet_events();
/// ```
pub struct BlockProcessor<E: EventSender = NoopEventSender> {
    /// Database ID of the account being processed.
    account_id: i64,
    /// The account's view key bytes for output ownership verification.
    account_view_key: Vec<u8>,
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
    /// * `account_id` - Database ID of the account to process blocks for
    /// * `account_view_key` - The account's view key bytes
    pub fn new(account_id: i64, account_view_key: Vec<u8>) -> Self {
        Self {
            account_id,
            account_view_key,
            wallet_events: Vec::new(),
            event_sender: NoopEventSender,
            current_block: None,
            current_tip_height: 0,
            has_pending_outbound: false,
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
    /// * `account_id` - Database ID of the account to process blocks for
    /// * `account_view_key` - The account's view key bytes
    /// * `event_sender` - Implementation of [`EventSender`] for event notifications
    /// * `has_pending_outbound` - Whether there are pending outbound transactions
    ///   that should be matched against scanned inputs
    pub fn with_event_sender(
        account_id: i64,
        account_view_key: Vec<u8>,
        event_sender: E,
        has_pending_outbound: bool,
    ) -> Self {
        Self {
            account_id,
            account_view_key,
            wallet_events: Vec::new(),
            event_sender,
            current_block: None,
            current_tip_height: 0,
            has_pending_outbound,
        }
    }

    /// Updates the pending outbound transaction flag.
    ///
    /// Set to `true` when there are pending outbound transactions that should
    /// be matched against scanned inputs to link them with blockchain data.
    pub fn set_has_pending_outbound(&mut self, value: bool) {
        self.has_pending_outbound = value;
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
    ///
    /// # Errors
    ///
    /// Returns [`BlockProcessorError`] if any database operation fails.
    pub async fn process_block(&mut self, tx: &Connection, block: &BlockScanResult) -> Result<(), BlockProcessorError> {
        self.current_tip_height = block.height;

        self.current_block = Some(BlockEventAccumulator::new(
            self.account_id,
            block.height,
            block.block_hash.to_vec(),
        ));

        self.process_outputs(tx, block)?;
        self.process_inputs(tx, block)?;
        self.record_scanned_block(tx, block)?;
        self.process_confirmations(tx, block)?;

        if let Some(acc) = self.current_block.take() {
            if !acc.outputs.is_empty() || !acc.inputs.is_empty() {
                self.save_and_emit_displayed_transactions(tx, block.height, &acc)
                    .await?;
            }

            let block_event = acc.build();
            self.event_sender.send(ProcessingEvent::BlockProcessed(block_event));
        }

        Ok(())
    }

    async fn save_and_emit_displayed_transactions(
        &self,
        tx: &Connection,
        block_height: u64,
        accumulator: &BlockEventAccumulator,
    ) -> Result<(), BlockProcessorError> {
        if accumulator.full_balance_changes.is_empty() {
            return Ok(());
        }

        let processor = DisplayedTransactionProcessor::new(self.current_tip_height);
        let context = ProcessingContext::InMemory {
            detected_outputs: &accumulator.outputs,
            spent_inputs: &accumulator.inputs,
        };

        match processor
            .process_balance_changes(accumulator.full_balance_changes.clone(), context)
            .await
        {
            Ok(transactions) if !transactions.is_empty() => {
                let mut transactions_to_emit = Vec::new();

                for transaction in transactions {
                    // Check if this is a scanned version of an existing pending outbound transaction
                    if let Some(existing_pending) = self.find_matching_pending_outbound(tx, &transaction)? {
                        // Update the existing pending transaction with blockchain info
                        let updated = self.merge_pending_with_scanned(existing_pending, &transaction, block_height);
                        db::update_displayed_transaction_mined(tx, &updated)?;
                        transactions_to_emit.push(updated);
                    } else {
                        db::insert_displayed_transaction(tx, &transaction)?;
                        transactions_to_emit.push(transaction);
                    }
                }

                self.event_sender
                    .send(ProcessingEvent::TransactionsReady(DisplayedTransactionsEvent {
                        account_id: self.account_id,
                        transactions: transactions_to_emit,
                        block_height: Some(block_height),
                        is_initial_sync: false,
                    }));
            },
            Ok(_) => {},
            Err(e) => {
                eprintln!(
                    "Failed to process displayed transactions for block {}: {}",
                    block_height, e
                );
            },
        }

        Ok(())
    }

    fn find_matching_pending_outbound(
        &self,
        tx: &Connection,
        scanned_transaction: &DisplayedTransaction,
    ) -> Result<Option<DisplayedTransaction>, BlockProcessorError> {
        // Skip DB lookup if no pending outbound transactions exist
        if !self.has_pending_outbound {
            return Ok(None);
        }

        // Only match for outgoing transactions (spent inputs)
        if scanned_transaction.direction != TransactionDirection::Outgoing {
            return Ok(None);
        }

        // Check if any of the scanned transaction's inputs match a pending outbound transaction
        for input in &scanned_transaction.details.inputs {
            if let Some(pending) = db::find_pending_outbound_by_output_hash(tx, self.account_id, &input.output_hash)? {
                return Ok(Some(pending));
            }
        }

        Ok(None)
    }

    fn merge_pending_with_scanned(
        &self,
        mut pending: DisplayedTransaction,
        scanned: &DisplayedTransaction,
        block_height: u64,
    ) -> DisplayedTransaction {
        // Update blockchain info from the scanned transaction
        pending.blockchain.block_height = block_height;
        pending.blockchain.timestamp = scanned.blockchain.timestamp;
        pending.blockchain.confirmations = scanned.blockchain.confirmations;

        // Update status based on confirmations
        if pending.blockchain.confirmations >= REQUIRED_CONFIRMATIONS {
            pending.status = TransactionDisplayStatus::Confirmed;
        } else if pending.blockchain.confirmations > 0 {
            pending.status = TransactionDisplayStatus::Unconfirmed;
        }

        // Merge any additional details from scanned transaction
        pending.details.outputs = scanned.details.outputs.clone();

        pending
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

    /// Returns the account ID this processor is handling.
    pub fn account_id(&self) -> i64 {
        self.account_id
    }

    /// Processes all detected outputs in the block.
    ///
    /// For each output owned by the wallet:
    /// - Inserts the output into the database
    /// - Creates a wallet event for detection
    /// - Records the balance change (credit)
    /// - Adds to the block accumulator for event emission
    fn process_outputs(&mut self, tx: &Connection, block: &BlockScanResult) -> Result<(), BlockProcessorError> {
        for (hash, output) in &block.wallet_outputs {
            let memo = MemoInfo::from_output(output);

            let event = self.make_output_detected_event(*hash, block, &memo);
            self.wallet_events.push(event.clone());

            let (output_id, is_new) = db::insert_output(
                tx,
                self.account_id,
                &self.account_view_key,
                hash.to_vec(),
                output,
                block.height,
                block.block_hash.as_slice(),
                block.mined_timestamp,
                memo.parsed.clone(),
                memo.hex.clone(),
            )?;

            if is_new {
                db::insert_wallet_event(tx, self.account_id, &event)?;

                let balance_change = self.record_output_balance_change(tx, output_id, block, output)?;

                if let Some(ref mut acc) = self.current_block {
                    acc.outputs.push(DetectedOutput {
                        hash: *hash,
                        value: output.value().as_u64(),
                        is_coinbase: output.features().is_coinbase(),
                        memo: memo.parsed,
                    });
                    acc.add_balance_change(balance_change);
                }
            }
        }

        Ok(())
    }

    /// Creates a wallet event for a newly detected output.
    fn make_output_detected_event(&self, hash: FixedHash, block: &BlockScanResult, memo: &MemoInfo) -> WalletEvent {
        WalletEvent {
            id: 0,
            account_id: self.account_id,
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
        output_id: i64,
        block: &BlockScanResult,
        output: &WalletOutput,
    ) -> Result<BalanceChange, BlockProcessorError> {
        let change =
            make_balance_change_for_output(self.account_id, output_id, block.mined_timestamp, block.height, output);

        db::insert_balance_change(tx, &change)?;

        Ok(change)
    }

    /// Processes all inputs in the block to detect spent outputs.
    ///
    /// For each input that references a wallet-owned output:
    /// - Inserts the input record into the database
    /// - Records the balance change (debit)
    /// - Updates the output status to Spent
    /// - Adds to the block accumulator for event emission
    fn process_inputs(&mut self, tx: &Connection, block: &BlockScanResult) -> Result<(), BlockProcessorError> {
        for input_hash in &block.inputs {
            let Some((output_id, value)) = db::get_output_info_by_hash(tx, input_hash.as_slice())? else {
                continue;
            };

            let (input_id, is_new) = db::insert_input(
                tx,
                self.account_id,
                output_id,
                block.height,
                block.block_hash.as_slice(),
                block.mined_timestamp,
            )?;

            if is_new {
                let balance_change = self.record_input_balance_change(tx, input_id, value, block)?;
                db::update_output_status(tx, output_id, OutputStatus::Spent)?;

                if let Some(ref mut acc) = self.current_block {
                    acc.inputs.push(SpentInput {
                        output_hash: input_hash.to_vec(),
                        value,
                    });
                    acc.add_balance_change(balance_change);
                }
            }
        }

        Ok(())
    }

    /// Records a balance change for a spent input (debit).
    fn record_input_balance_change(
        &self,
        tx: &Connection,
        input_id: i64,
        value: u64,
        block: &BlockScanResult,
    ) -> Result<BalanceChange, BlockProcessorError> {
        let effective_date = DateTime::<Utc>::from_timestamp(block.mined_timestamp as i64, 0)
            .unwrap_or_else(Utc::now)
            .naive_utc();

        let change = BalanceChange {
            account_id: self.account_id,
            caused_by_output_id: None,
            caused_by_input_id: Some(input_id),
            description: "Output spent as input".to_string(),
            balance_credit: 0,
            balance_debit: value,
            effective_date,
            effective_height: block.height,
            claimed_recipient_address: None,
            claimed_sender_address: None,
            memo_hex: None,
            memo_parsed: None,
            claimed_fee: None,
            claimed_amount: None,
        };

        db::insert_balance_change(tx, &change)?;
        Ok(change)
    }

    /// Records the scanned block for reorg detection.
    ///
    /// Stores the block height and hash in the scanned_tip_blocks table,
    /// which is used to detect chain reorganizations.
    fn record_scanned_block(&self, tx: &Connection, block: &BlockScanResult) -> Result<(), BlockProcessorError> {
        db::insert_scanned_tip_block(tx, self.account_id, block.height as i64, block.block_hash.as_slice())?;
        Ok(())
    }

    /// Processes confirmation status updates for maturing outputs.
    ///
    /// Finds outputs that have reached the required confirmation depth
    /// and updates their status to confirmed.
    fn process_confirmations(&mut self, tx: &Connection, block: &BlockScanResult) -> Result<(), BlockProcessorError> {
        let unconfirmed_outputs =
            db::get_unconfirmed_outputs(tx, self.account_id, block.height, REQUIRED_CONFIRMATIONS)?;

        for unconfirmed_output in unconfirmed_outputs {
            let event = self.make_confirmation_event(
                &unconfirmed_output.output_hash,
                unconfirmed_output.mined_in_block_height as u64,
                block.height,
                unconfirmed_output.memo_parsed,
                unconfirmed_output.memo_hex,
            );

            self.wallet_events.push(event.clone());
            db::insert_wallet_event(tx, self.account_id, &event)?;

            db::mark_output_confirmed(
                tx,
                &unconfirmed_output.output_hash,
                block.height,
                block.block_hash.as_slice(),
            )?;

            if let Some(ref mut acc) = self.current_block {
                acc.confirmations.push(ConfirmedOutput {
                    hash: unconfirmed_output.output_hash.clone(),
                    original_height: unconfirmed_output.mined_in_block_height as u64,
                    confirmation_height: block.height,
                });
            }
        }

        Ok(())
    }

    /// Creates a wallet event for an output reaching confirmation depth.
    fn make_confirmation_event(
        &self,
        output_hash: &[u8],
        original_height: u64,
        confirmation_height: u64,
        memo_parsed: Option<String>,
        memo_hex: Option<String>,
    ) -> WalletEvent {
        WalletEvent {
            id: 0,
            account_id: self.account_id,
            event_type: WalletEventType::OutputConfirmed {
                hash: output_hash.to_vec(),
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

/// Accumulates events and data for a single block during processing.
///
/// This struct collects all outputs, inputs, confirmations, and balance changes
/// detected while processing a block, then builds them into a single
/// [`BlockProcessedEvent`] for emission.
struct BlockEventAccumulator {
    /// Account ID for the block being processed.
    account_id: i64,
    /// Block height.
    height: u64,
    /// Block hash bytes.
    block_hash: Vec<u8>,
    /// Outputs detected in this block.
    outputs: Vec<DetectedOutput>,
    /// Inputs (spent outputs) detected in this block.
    inputs: Vec<SpentInput>,
    /// Outputs that reached confirmation depth at this block.
    confirmations: Vec<ConfirmedOutput>,
    /// Complete balance change records for transaction processing.
    full_balance_changes: Vec<BalanceChange>,
}

impl BlockEventAccumulator {
    /// Creates a new accumulator for the given block.
    fn new(account_id: i64, height: u64, block_hash: Vec<u8>) -> Self {
        Self {
            account_id,
            height,
            block_hash,
            outputs: Vec::new(),
            inputs: Vec::new(),
            confirmations: Vec::new(),
            full_balance_changes: Vec::new(),
        }
    }

    /// Adds a balance change to the accumulator.
    fn add_balance_change(&mut self, change: BalanceChange) {
        self.full_balance_changes.push(change);
    }

    /// Consumes the accumulator and builds the final [`BlockProcessedEvent`].
    fn build(self) -> BlockProcessedEvent {
        let balance_changes = self
            .full_balance_changes
            .iter()
            .map(|c| BalanceChangeSummary {
                credit: c.balance_credit,
                debit: c.balance_debit,
                description: c.description.clone(),
            })
            .collect();

        BlockProcessedEvent {
            account_id: self.account_id,
            height: self.height,
            block_hash: self.block_hash,
            outputs_detected: self.outputs,
            inputs_spent: self.inputs,
            outputs_confirmed: self.confirmations,
            balance_changes,
        }
    }
}

/// Extracted memo information from an output's payment ID.
struct MemoInfo {
    /// Human-readable interpretation of the memo (UTF-8 lossy).
    parsed: Option<String>,
    /// Hexadecimal encoding of the raw memo bytes.
    hex: Option<String>,
}

impl MemoInfo {
    /// Extracts memo information from a wallet output's payment ID.
    fn from_output(output: &WalletOutput) -> Self {
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
            balance_credit: output.value().as_u64(),
            balance_debit: 0,
            effective_date,
            effective_height: height,
            claimed_recipient_address: None,
            claimed_sender_address: None,
            memo_parsed: Some(String::from_utf8_lossy(&memo_bytes).to_string()),
            memo_hex: Some(hex::encode(&memo_bytes)),
            claimed_fee: payment_info.get_fee().map(|v| v.0),
            claimed_amount: payment_info.get_amount().map(|v| v.0),
        };
    }

    BalanceChange {
        account_id,
        caused_by_output_id: Some(output_id),
        caused_by_input_id: None,
        description: "Output found in blockchain scan".to_string(),
        balance_credit: output.value().as_u64(),
        balance_debit: 0,
        effective_date,
        effective_height: height,
        claimed_recipient_address: payment_info.get_recipient_address().map(|a| a.to_base58()),
        claimed_sender_address: payment_info.get_sender_address().map(|a| a.to_base58()),
        memo_parsed: Some(String::from_utf8_lossy(&memo_bytes).to_string()),
        memo_hex: Some(hex::encode(&memo_bytes)),
        claimed_fee: payment_info.get_fee().map(|v| v.0),
        claimed_amount: payment_info.get_amount().map(|v| v.0),
    }
}
