use crate::models::{Id, OutputStatus};
use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use tari_common_types::payment_reference::PaymentReference;
use tari_common_types::tari_address::TariAddress;
use tari_common_types::transaction::TxId;
use tari_common_types::types::FixedHash;
use tari_transaction_components::MicroMinotari;
use tari_transaction_components::transaction_components::{CoinBaseExtra, OutputType};
use utoipa::openapi::{Object, Schema, Type};
use utoipa::openapi::schema::SchemaType;
use utoipa::ToSchema;


pub fn micro_minotari_schema() -> Schema {
    Schema::Object(Object::with_type(SchemaType::Type(Type::Integer)))
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
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
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DisplayedTransaction {
    pub id: TxId,
    pub direction: TransactionDirection,
    pub source: TransactionSource,
    pub status: TransactionDisplayStatus,
    /// Net amount in microTari (always positive, use direction for sign).
    pub amount: MicroMinotari,
    pub message: Option<String>,
    pub counterparty: Option<TariAddress>,
    pub blockchain: BlockchainInfo,
    /// Fee information (only populated for outgoing transactions).
    pub fee: Option<FeeInfo>,
    pub details: TransactionDetails,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CounterpartyInfo {
    pub address: String,
    pub address_emoji: Option<String>,
    /// User-defined alias from address book.
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BlockchainInfo {
    pub block_height: u64,
    #[schema(value_type = String)]
    pub timestamp: NaiveDateTime,
    pub confirmations: u64,
    pub block_hash: FixedHash,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct FeeInfo {
    pub amount: MicroMinotari,
}

/// Advanced transaction details.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TransactionDetails {
    #[schema(value_type = i64)]
    pub account_id: Id,
    #[schema(schema_with = micro_minotari_schema)]
    pub total_credit: MicroMinotari,
    #[schema(schema_with = micro_minotari_schema)]
    pub total_debit: MicroMinotari,
    pub inputs: Vec<TransactionInput>,
    pub outputs: Vec<TransactionOutput>,
    pub output_type: Option<OutputType>,
    /// Extra data for coinbase transactions.
    pub coinbase_extra: Option<CoinBaseExtra>,
    pub memo_hex: Option<String>,
    /// Hashes of outputs sent in this transaction (hex encoded).
    /// Used to match pending broadcasted transactions with scanned ones.
    pub sent_output_hashes: Vec<FixedHash>,
    pub sent_payrefs: Vec<PaymentReference>,
}

/// A transaction input (spent UTXO).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TransactionInput {
    pub output_hash: FixedHash,
    pub amount: MicroMinotari,
    /// ID of the matched output in our database (if found).
    #[schema(value_type = Option<i64>)]
    pub matched_output_id: Option<Id>,
    pub is_matched: bool,
    pub mined_in_block_hash: FixedHash,
}

/// A transaction output (created UTXO).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TransactionOutput {
    pub hash: FixedHash,
    pub amount: MicroMinotari,
    pub status: OutputStatus,
    pub mined_in_block_height: u64,
    pub mined_in_block_hash: FixedHash,
    pub output_type: OutputType,
    pub is_change: bool,
}
