mod database;
mod in_memory;

use std::collections::HashMap;

use async_trait::async_trait;

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
    pub hash_hex: String,
    pub confirmed_height: Option<u64>,
    pub status: OutputStatus,
    pub output_type: String,
    pub coinbase_extra: Option<String>,
    pub is_coinbase: bool,
    pub sent_output_hashes: Vec<String>,
}

/// Trait for resolving transaction data from different sources.
#[async_trait]
pub trait TransactionDataResolver: Send + Sync {
    async fn get_output_details(&self, change: &BalanceChange) -> Result<Option<OutputDetails>, ProcessorError>;

    async fn get_input_output_hash(&self, change: &BalanceChange) -> Result<Option<String>, ProcessorError>;

    async fn get_sent_output_hashes(&self, change: &BalanceChange) -> Result<Vec<String>, ProcessorError>;

    async fn build_output_hash_map(&self) -> Result<HashMap<String, Id>, ProcessorError>;
}
