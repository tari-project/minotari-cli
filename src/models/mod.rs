use lightweight_wallet_libs::transaction_components::WalletOutput;

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
    pub id: Id,
    pub account_id: Id,
    pub event_type: WalletEventType,
    pub details: String,
}

#[derive(Debug, Clone)]
pub enum WalletEventType {
    BlockRolledBack,
    OutputDetected { output: WalletOutput },
    OutputRolledBack,
}
