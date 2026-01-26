use crate::models::{Id, OutputStatus};
use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use tari_common_types::payment_reference::PaymentReference;
use tari_common_types::types::FixedHash;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TransactionDirection {
    Incoming,
    Outgoing,
}

impl TransactionDirection {
    pub fn as_label(&self) -> &'static str {
        match self {
            Self::Incoming => "Received",
            Self::Outgoing => "Sent",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TransactionSource {
    Transfer,
    Coinbase,
    OneSided,
    Unknown,
}

impl TransactionSource {
    pub fn as_label(&self) -> &'static str {
        match self {
            Self::Transfer => "Transfer",
            Self::Coinbase => "Mining Reward",
            Self::OneSided => "One-sided Payment",
            Self::Unknown => "Transaction",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TransactionDisplayStatus {
    Pending,
    Unconfirmed,
    Confirmed,
    Cancelled,
    Reorganized,
    Rejected,
}

impl TransactionDisplayStatus {
    pub fn as_label(&self) -> &'static str {
        match self {
            Self::Pending => "Pending",
            Self::Unconfirmed => "Unconfirmed",
            Self::Confirmed => "Confirmed",
            Self::Cancelled => "Cancelled",
            Self::Reorganized => "Reorganized",
            Self::Rejected => "Rejected",
        }
    }
}

/// User-friendly transaction representation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisplayedTransaction {
    pub id: String,
    pub direction: TransactionDirection,
    pub source: TransactionSource,
    pub status: TransactionDisplayStatus,
    /// Net amount in microTari (always positive, use direction for sign).
    pub amount: u64,
    /// User-friendly amount (e.g., "1,234.567890 XTM").
    pub amount_display: String,
    pub message: Option<String>,
    pub counterparty: Option<CounterpartyInfo>,
    pub blockchain: BlockchainInfo,
    /// Fee information (only populated for outgoing transactions).
    pub fee: Option<FeeInfo>,
    pub details: TransactionDetails,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CounterpartyInfo {
    pub address: String,
    pub address_emoji: Option<String>,
    /// User-defined alias from address book.
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockchainInfo {
    pub block_height: u64,
    pub timestamp: NaiveDateTime,
    pub confirmations: u64,
    pub block_hash: FixedHash,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeeInfo {
    pub amount: u64,
    pub amount_display: String,
}

/// Advanced transaction details.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionDetails {
    pub account_id: Id,
    pub total_credit: u64,
    pub total_debit: u64,
    pub inputs: Vec<TransactionInput>,
    pub outputs: Vec<TransactionOutput>,
    pub output_type: Option<String>,
    /// Extra data for coinbase transactions.
    pub coinbase_extra: Option<String>,
    pub memo_hex: Option<String>,
    /// Hashes of outputs sent in this transaction (hex encoded).
    /// Used to match pending broadcasted transactions with scanned ones.
    #[serde(default)]
    pub sent_output_hashes: Vec<FixedHash>,
    #[serde(default)]
    pub sent_payrefs: Vec<PaymentReference>,
}

/// A transaction input (spent UTXO).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionInput {
    pub output_hash: FixedHash,
    pub amount: u64,
    /// ID of the matched output in our database (if found).
    pub matched_output_id: Option<Id>,
    pub is_matched: bool,
    pub mined_in_block_hash: FixedHash,
}

/// A transaction output (created UTXO).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionOutput {
    pub hash: FixedHash,
    pub amount: u64,
    pub status: OutputStatus,
    pub mined_in_block_height: u64,
    pub mined_in_block_hash: FixedHash,
    pub output_type: String,
    pub is_change: bool,
}
