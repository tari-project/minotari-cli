use chrono::{DateTime, Utc};
use lightweight_wallet_libs::BlockScanResult;
use sqlx::SqliteConnection;
use tari_common_types::types::FixedHash;
use tari_transaction_components::transaction_components::WalletOutput;
use thiserror::Error;

use crate::{
    db,
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

#[derive(Debug, Error)]
pub enum BlockProcessorError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("Failed to insert wallet event: {0}")]
    WalletEvent(#[from] anyhow::Error),
}

/// Processes scanned blocks and persists wallet data.
pub struct BlockProcessor<E: EventSender = NoopEventSender> {
    account_id: i64,
    wallet_events: Vec<WalletEvent>,
    event_sender: E,
    current_block: Option<BlockEventAccumulator>,
    current_tip_height: u64,
    has_pending_outbound: bool,
}

impl BlockProcessor<NoopEventSender> {
    pub fn new(account_id: i64) -> Self {
        Self {
            account_id,
            wallet_events: Vec::new(),
            event_sender: NoopEventSender,
            current_block: None,
            current_tip_height: 0,
            has_pending_outbound: false,
        }
    }
}

impl<E: EventSender> BlockProcessor<E> {
    pub fn with_event_sender(account_id: i64, event_sender: E, has_pending_outbound: bool) -> Self {
        Self {
            account_id,
            wallet_events: Vec::new(),
            event_sender,
            current_block: None,
            current_tip_height: 0,
            has_pending_outbound,
        }
    }

    pub fn set_has_pending_outbound(&mut self, value: bool) {
        self.has_pending_outbound = value;
    }

    pub async fn process_block(
        &mut self,
        tx: &mut SqliteConnection,
        block: &BlockScanResult,
    ) -> Result<(), BlockProcessorError> {
        self.current_tip_height = block.height;

        self.current_block = Some(BlockEventAccumulator::new(
            self.account_id,
            block.height,
            block.block_hash.to_vec(),
        ));

        self.process_outputs(tx, block).await?;
        self.process_inputs(tx, block).await?;
        self.record_scanned_block(tx, block).await?;
        self.process_confirmations(tx, block).await?;

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
        tx: &mut SqliteConnection,
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
                    if let Some(existing_pending) = self.find_matching_pending_outbound(tx, &transaction).await? {
                        // Update the existing pending transaction with blockchain info
                        let updated = self.merge_pending_with_scanned(existing_pending, &transaction, block_height);
                        db::update_displayed_transaction_mined(tx, &updated).await?;
                        transactions_to_emit.push(updated);
                    } else {
                        db::insert_displayed_transaction(tx, &transaction).await?;
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

    async fn find_matching_pending_outbound(
        &self,
        tx: &mut SqliteConnection,
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
            if let Some(pending) =
                db::find_pending_outbound_by_output_hash(tx, self.account_id, &input.output_hash).await?
            {
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

    pub fn emit_status(&self, status: ScanStatusEvent) {
        self.event_sender.send(ProcessingEvent::ScanStatus(status));
    }

    pub fn into_wallet_events(self) -> Vec<WalletEvent> {
        self.wallet_events
    }

    pub fn account_id(&self) -> i64 {
        self.account_id
    }

    async fn process_outputs(
        &mut self,
        tx: &mut SqliteConnection,
        block: &BlockScanResult,
    ) -> Result<(), BlockProcessorError> {
        for (hash, output) in &block.wallet_outputs {
            let memo = MemoInfo::from_output(output);

            let event = self.make_output_detected_event(*hash, block, &memo);
            self.wallet_events.push(event.clone());

            let (output_id, is_new) = db::insert_output(
                tx,
                self.account_id,
                hash.to_vec(),
                output,
                block.height,
                block.block_hash.as_slice(),
                block.mined_timestamp,
                memo.parsed.clone(),
                memo.hex.clone(),
            )
            .await?;

            if is_new {
                db::insert_wallet_event(tx, self.account_id, &event).await?;

                let balance_change = self.record_output_balance_change(tx, output_id, block, output).await?;

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

    async fn record_output_balance_change(
        &self,
        tx: &mut SqliteConnection,
        output_id: i64,
        block: &BlockScanResult,
        output: &WalletOutput,
    ) -> Result<BalanceChange, BlockProcessorError> {
        let change =
            make_balance_change_for_output(self.account_id, output_id, block.mined_timestamp, block.height, output);

        db::insert_balance_change(tx, &change).await?;

        Ok(change)
    }

    async fn process_inputs(
        &mut self,
        tx: &mut SqliteConnection,
        block: &BlockScanResult,
    ) -> Result<(), BlockProcessorError> {
        for input_hash in &block.inputs {
            let Some((output_id, value)) = db::get_output_info_by_hash(tx, input_hash.as_slice()).await? else {
                continue;
            };

            let (input_id, is_new) = db::insert_input(
                tx,
                self.account_id,
                output_id,
                block.height,
                block.block_hash.as_slice(),
                block.mined_timestamp,
            )
            .await?;

            if is_new {
                let balance_change = self.record_input_balance_change(tx, input_id, value, block).await?;
                db::update_output_status(tx, output_id, OutputStatus::Spent).await?;

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

    async fn record_input_balance_change(
        &self,
        tx: &mut SqliteConnection,
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

        db::insert_balance_change(tx, &change).await?;
        Ok(change)
    }

    async fn record_scanned_block(
        &self,
        tx: &mut SqliteConnection,
        block: &BlockScanResult,
    ) -> Result<(), BlockProcessorError> {
        db::insert_scanned_tip_block(tx, self.account_id, block.height as i64, block.block_hash.as_slice()).await?;
        Ok(())
    }

    async fn process_confirmations(
        &mut self,
        tx: &mut SqliteConnection,
        block: &BlockScanResult,
    ) -> Result<(), BlockProcessorError> {
        let unconfirmed =
            db::get_unconfirmed_outputs(tx, self.account_id, block.height, REQUIRED_CONFIRMATIONS).await?;

        for (output_hash, original_height, memo_parsed, memo_hex) in unconfirmed {
            let event =
                self.make_confirmation_event(&output_hash, original_height, block.height, memo_parsed, memo_hex);

            self.wallet_events.push(event.clone());
            db::insert_wallet_event(tx, self.account_id, &event).await?;

            db::mark_output_confirmed(tx, &output_hash, block.height, block.block_hash.as_slice()).await?;

            if let Some(ref mut acc) = self.current_block {
                acc.confirmations.push(ConfirmedOutput {
                    hash: output_hash.clone(),
                    original_height,
                    confirmation_height: block.height,
                });
            }
        }

        Ok(())
    }

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

struct BlockEventAccumulator {
    account_id: i64,
    height: u64,
    block_hash: Vec<u8>,
    outputs: Vec<DetectedOutput>,
    inputs: Vec<SpentInput>,
    confirmations: Vec<ConfirmedOutput>,
    full_balance_changes: Vec<BalanceChange>,
}

impl BlockEventAccumulator {
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

    fn add_balance_change(&mut self, change: BalanceChange) {
        self.full_balance_changes.push(change);
    }

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

struct MemoInfo {
    parsed: Option<String>,
    hex: Option<String>,
}

impl MemoInfo {
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
