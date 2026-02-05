use super::{OutputDetails, TransactionDataResolver};
use crate::models::{BalanceChange, Id, OutputStatus};
use crate::scan::{DetectedOutput, SpentInput};
use crate::transactions::ProcessorError;
use std::collections::HashMap;
use tari_common_types::types::FixedHash;
use tari_transaction_components::MicroMinotari;
use tari_transaction_components::transaction_components::{CoinBaseExtra, OutputType};

/// Resolver that uses in-memory data from block processing.
pub struct InMemoryResolver<'a> {
    detected_outputs: &'a [DetectedOutput],
    spent_inputs: &'a [SpentInput],
    output_by_value: HashMap<MicroMinotari, &'a DetectedOutput>,
    input_by_value: HashMap<MicroMinotari, &'a SpentInput>,
    output_hashes: std::collections::HashSet<FixedHash>,
}

impl<'a> InMemoryResolver<'a> {
    pub fn new(detected_outputs: &'a [DetectedOutput], spent_inputs: &'a [SpentInput]) -> Self {
        let output_by_value: HashMap<MicroMinotari, &DetectedOutput> =
            detected_outputs.iter().map(|o| (o.output.value(), o)).collect();

        let input_by_value: HashMap<MicroMinotari, &SpentInput> =
            spent_inputs.iter().map(|i| (i.output.value(), i)).collect();

        let output_hashes: std::collections::HashSet<FixedHash> =
            detected_outputs.iter().map(|o| o.output.output_hash()).collect();

        Self {
            detected_outputs,
            spent_inputs,
            output_by_value,
            input_by_value,
            output_hashes,
        }
    }

    #[allow(dead_code)]
    pub fn detected_outputs(&self) -> &[DetectedOutput] {
        self.detected_outputs
    }

    #[allow(dead_code)]
    pub fn spent_inputs(&self) -> &[SpentInput] {
        self.spent_inputs
    }

    #[allow(dead_code)]
    pub fn contains_output_hash(&self, hash: &FixedHash) -> bool {
        self.output_hashes.contains(hash)
    }
}

impl TransactionDataResolver for InMemoryResolver<'_> {
    fn get_output_details(&self, change: &BalanceChange) -> Result<Option<OutputDetails>, ProcessorError> {
        if change.balance_credit == 0.into() {
            return Ok(None);
        }

        if let Some(output) = self.output_by_value.get(&change.balance_credit) {
            return Ok(Some(OutputDetails {
                hash: output.output.output_hash(),
                mined_in_block_height: output.height,
                mined_hash: output.mined_in_block_hash,
                status: OutputStatus::Unspent,
                output_type: if output.output.is_coinbase() {
                    OutputType::Coinbase
                } else {
                    OutputType::Standard
                },
                coinbase_extra: CoinBaseExtra::default(),
                sent_output_hashes: Vec::new(),
            }));
        }

        Ok(None)
    }

    fn get_input_output_hash(&self, change: &BalanceChange) -> Result<Option<(FixedHash, FixedHash)>, ProcessorError> {
        if change.balance_debit == 0.into() {
            return Ok(None);
        }

        if let Some(input) = self.input_by_value.get(&change.balance_debit) {
            return Ok(Some((input.output.output_hash(), input.mined_in_block)));
        }

        Ok(None)
    }

    fn get_sent_output_hashes(&self, _change: &BalanceChange) -> Result<Vec<FixedHash>, ProcessorError> {
        Ok(Vec::new())
    }

    fn build_output_hash_map(&self) -> Result<HashMap<FixedHash, Id>, ProcessorError> {
        Ok(HashMap::new())
    }
}
