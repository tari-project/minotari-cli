mod database;
mod in_memory;

use std::collections::HashMap;
use tari_common_types::types::FixedHash;
use super::error::ProcessorError;
use crate::models::{BalanceChange, Id, OutputStatus};

pub use self::context::ProcessingContext;
pub use database::DatabaseResolver;
pub use in_memory::InMemoryResolver;

mod context {
    use crate::{
        db::SqlitePool,
        scan::{DetectedOutput, SpentInput},
    };

    pub enum ProcessingContext<'a> {
        Database(&'a SqlitePool),
        InMemory {
            detected_outputs: &'a [DetectedOutput],
            spent_inputs: &'a [SpentInput],
        },
    }
}

#[derive(Debug, Clone)]
pub struct OutputDetails {
    pub hash: FixedHash,
    pub mined_in_block_height: u64,
    pub mined_hash: FixedHash,
    pub status: OutputStatus,
    pub output_type: String,
    pub coinbase_extra: Option<String>,
    pub is_coinbase: bool,
    pub sent_output_hashes: Vec<FixedHash>,
}

/// Trait for resolving transaction data from different sources.
pub trait TransactionDataResolver: Send + Sync {
    fn get_output_details(&self, change: &BalanceChange) -> Result<Option<OutputDetails>, ProcessorError>;

    fn get_input_output_hash(&self, change: &BalanceChange) -> Result<Option<(FixedHash, FixedHash)>, ProcessorError>;

    fn get_sent_output_hashes(&self, change: &BalanceChange) -> Result<Vec<FixedHash>, ProcessorError>;

    fn build_output_hash_map(&self) -> Result<HashMap<FixedHash, Id>, ProcessorError>;
}
