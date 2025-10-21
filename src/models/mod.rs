use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use tari_common_types::types::FixedHash;

pub mod output_status;
pub use output_status::OutputStatus;

// Change depending on sql type.
pub type Id = i64;

#[derive(Debug, Clone)]
pub struct ScannedTipBlock {
    pub id: Id,
    pub account_id: Id,
    pub height: u64,
    pub hash: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct WalletEvent {
    #[allow(dead_code)]
    pub id: Id,
    #[allow(dead_code)]
    pub account_id: Id,
    pub event_type: WalletEventType,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WalletEventType {
    BlockRolledBack,
    OutputDetected {
        hash: FixedHash,
        block_height: u64,
        block_hash: Vec<u8>,
        memo_parsed: Option<String>,
        memo_hex: Option<String>,
    },
    OutputConfirmed {
        hash: Vec<u8>,
        block_height: u64,
        confirmation_height: u64,
        memo_parsed: Option<String>,
        memo_hex: Option<String>,
    },
    OutputRolledBack,
}

impl WalletEventType {
    pub fn to_key_string(&self) -> String {
        match &self {
            WalletEventType::BlockRolledBack => "BlockRolledBack".to_string(),
            WalletEventType::OutputDetected { .. } => "OutputDetected".to_string(),
            WalletEventType::OutputConfirmed { .. } => "OutputConfirmed".to_string(),
            WalletEventType::OutputRolledBack => "OutputRolledBack".to_string(),
        }
    }
}

pub struct BalanceChange {
    pub account_id: Id,
    pub caused_by_output_id: Option<Id>,
    pub caused_by_input_id: Option<Id>,
    pub description: String,
    pub balance_credit: u64,
    pub balance_debit: u64,
    pub effective_date: NaiveDateTime,
    pub effective_height: u64,
    pub claimed_recipient_address: Option<String>,
    pub claimed_sender_address: Option<String>,
    pub memo_parsed: Option<String>,
    pub memo_hex: Option<String>,
    pub claimed_fee: Option<u64>,
    pub claimed_amount: Option<u64>,
}
