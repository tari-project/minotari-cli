// Copyright 2025 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use serde::{Deserialize, Serialize};
use std::fmt::Display;

#[derive(Debug, Serialize, Deserialize)]
pub struct JsonRpcResponse<T> {
    pub result: Option<T>,
    pub error: Option<String>,
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TipInfoResponse {
    pub metadata: Option<ChainMetadata>,
    pub is_synced: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainMetadata {
    pub best_block_height: u64,
    pub best_block_hash: Vec<u8>,
    pub pruning_horizon: u64,
    pub pruned_height: u64,
    pub accumulated_difficulty: u64,
    pub timestamp: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TxSubmissionResponse {
    pub accepted: bool,
    pub rejection_reason: TxSubmissionRejectionReason,
    pub is_synced: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TxSubmissionRejectionReason {
    None,
    AlreadyMined,
    DoubleSpend,
    Orphan,
    TimeLocked,
    ValidationFailed,
    FeeTooLow,
}

impl Display for TxSubmissionRejectionReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TxSubmissionRejectionReason::None => write!(f, "None"),
            TxSubmissionRejectionReason::AlreadyMined => write!(f, "Already Mined"),
            TxSubmissionRejectionReason::DoubleSpend => write!(f, "Double Spend"),
            TxSubmissionRejectionReason::Orphan => write!(f, "Orphan"),
            TxSubmissionRejectionReason::TimeLocked => write!(f, "Time Locked"),
            TxSubmissionRejectionReason::ValidationFailed => write!(f, "Validation Failed"),
            TxSubmissionRejectionReason::FeeTooLow => write!(f, "Fee Too Low"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TxLocation {
    None = 0,
    NotStored = 1,
    InMempool = 2,
    Mined = 3,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TxQueryResponse {
    pub location: TxLocation,
    pub mined_height: Option<u64>,
    pub mined_header_hash: Option<Vec<u8>>,
    pub mined_timestamp: Option<u64>,
}
