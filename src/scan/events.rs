use std::time::Duration;

use tari_common_types::types::FixedHash;

use crate::transactions::DisplayedTransaction;

#[derive(Debug, Clone)]
pub enum ProcessingEvent {
    BlockProcessed(BlockProcessedEvent),
    ScanStatus(ScanStatusEvent),
    TransactionsReady(DisplayedTransactionsEvent),
    TransactionsUpdated(TransactionsUpdatedEvent),
    ReorgDetected(ReorgDetectedEvent),
}

#[derive(Debug, Clone)]
pub struct DisplayedTransactionsEvent {
    pub account_id: i64,
    pub transactions: Vec<DisplayedTransaction>,
    pub block_height: Option<u64>,
    pub is_initial_sync: bool,
}

#[derive(Debug, Clone)]
pub struct TransactionsUpdatedEvent {
    pub account_id: i64,
    pub updated_transactions: Vec<DisplayedTransaction>,
}

#[derive(Debug, Clone)]
pub struct BlockProcessedEvent {
    pub account_id: i64,
    pub height: u64,
    pub block_hash: Vec<u8>,
    pub outputs_detected: Vec<DetectedOutput>,
    pub inputs_spent: Vec<SpentInput>,
    pub outputs_confirmed: Vec<ConfirmedOutput>,
    pub balance_changes: Vec<BalanceChangeSummary>,
}

#[derive(Debug, Clone)]
pub struct DetectedOutput {
    pub hash: FixedHash,
    pub value: u64,
    pub is_coinbase: bool,
    pub memo: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SpentInput {
    pub output_hash: Vec<u8>,
    pub value: u64,
}

#[derive(Debug, Clone)]
pub struct ConfirmedOutput {
    pub hash: Vec<u8>,
    pub original_height: u64,
    pub confirmation_height: u64,
}

#[derive(Debug, Clone)]
pub struct BalanceChangeSummary {
    pub credit: u64,
    pub debit: u64,
    pub description: String,
}

#[derive(Debug, Clone)]
pub enum ScanStatusEvent {
    Started {
        account_id: i64,
        from_height: u64,
    },
    Progress {
        account_id: i64,
        current_height: u64,
        blocks_scanned: u64,
    },
    MoreBlocksAvailable {
        account_id: i64,
        last_scanned_height: u64,
    },
    Completed {
        account_id: i64,
        final_height: u64,
        total_blocks_scanned: u64,
    },
    Waiting {
        account_id: i64,
        resume_in: Duration,
    },
    Paused {
        account_id: i64,
        last_scanned_height: u64,
        reason: PauseReason,
    },
}

#[derive(Debug, Clone)]
pub enum PauseReason {
    MaxBlocksReached { limit: u64 },
    Cancelled,
}

#[derive(Debug, Clone)]
pub struct ReorgDetectedEvent {
    pub account_id: i64,
    pub reorg_from_height: u64,
    pub new_height: u64,
    pub blocks_rolled_back: u64,
    pub invalidated_output_hashes: Vec<FixedHash>,
    pub cancelled_transaction_ids: Vec<String>,
    pub reorganized_displayed_transactions: Vec<DisplayedTransaction>,
}

pub trait EventSender: Send + Sync {
    fn send(&self, event: ProcessingEvent) -> bool;
}

#[derive(Debug, Clone, Default)]
pub struct NoopEventSender;

impl EventSender for NoopEventSender {
    fn send(&self, _event: ProcessingEvent) -> bool {
        true
    }
}

#[derive(Debug, Clone)]
pub struct ChannelEventSender {
    sender: tokio::sync::mpsc::UnboundedSender<ProcessingEvent>,
}

impl ChannelEventSender {
    pub fn new(sender: tokio::sync::mpsc::UnboundedSender<ProcessingEvent>) -> Self {
        Self { sender }
    }
}

impl EventSender for ChannelEventSender {
    fn send(&self, event: ProcessingEvent) -> bool {
        self.sender.send(event).is_ok()
    }
}
