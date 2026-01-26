use super::error::ProcessorError;
use super::formatting::format_micro_tari;
use super::types::{
    BlockchainInfo, CounterpartyInfo, DisplayedTransaction, FeeInfo, TransactionDetails, TransactionDirection,
    TransactionDisplayStatus, TransactionInput, TransactionOutput, TransactionSource,
};
use crate::models::Id;
use chrono::NaiveDateTime;
use tari_common_types::payment_reference::generate_payment_reference;
use tari_common_types::types::FixedHash;

#[derive(Debug, Default)]
pub struct DisplayedTransactionBuilder {
    id: Option<String>,
    account_id: Option<Id>,
    direction: Option<TransactionDirection>,
    source: Option<TransactionSource>,
    status: Option<TransactionDisplayStatus>,
    amount: Option<u64>,
    message: Option<String>,
    counterparty_address: Option<String>,
    counterparty_emoji: Option<String>,
    block_height: Option<u64>,
    block_hash: Option<FixedHash>,
    timestamp: Option<NaiveDateTime>,
    confirmations: Option<u64>,
    fee: Option<u64>,
    total_credit: u64,
    total_debit: u64,
    inputs: Vec<TransactionInput>,
    outputs: Vec<TransactionOutput>,
    output_type: Option<String>,
    coinbase_extra: Option<String>,
    memo_hex: Option<String>,
    sent_output_hashes: Vec<FixedHash>,
}

impl DisplayedTransactionBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    pub fn account_id(mut self, id: Id) -> Self {
        self.account_id = Some(id);
        self
    }

    pub fn direction(mut self, direction: TransactionDirection) -> Self {
        self.direction = Some(direction);
        self
    }

    pub fn source(mut self, source: TransactionSource) -> Self {
        self.source = Some(source);
        self
    }

    pub fn status(mut self, status: TransactionDisplayStatus) -> Self {
        self.status = Some(status);
        self
    }

    /// Set credits and debits, auto-calculating amount and direction.
    pub fn credits_and_debits(mut self, credit: u64, debit: u64) -> Self {
        self.total_credit = credit;
        self.total_debit = debit;

        if debit > credit {
            self.amount = Some(debit.saturating_sub(credit));
            self.direction = Some(TransactionDirection::Outgoing);
        } else {
            self.amount = Some(credit.saturating_sub(debit));
            self.direction = Some(TransactionDirection::Incoming);
        }

        self
    }

    pub fn message(mut self, message: Option<String>) -> Self {
        self.message = message;
        self
    }

    pub fn counterparty(mut self, address: Option<String>, emoji: Option<String>) -> Self {
        self.counterparty_address = address;
        self.counterparty_emoji = emoji;
        self
    }

    pub fn blockchain_info(
        mut self,
        height: u64,
        hash: FixedHash,
        timestamp: NaiveDateTime,
        confirmations: u64,
    ) -> Self {
        self.block_height = Some(height);
        self.timestamp = Some(timestamp);
        self.confirmations = Some(confirmations);
        self.block_hash = Some(hash);
        self
    }

    pub fn fee(mut self, fee: Option<u64>) -> Self {
        self.fee = fee;
        self
    }

    pub fn inputs(mut self, inputs: Vec<TransactionInput>) -> Self {
        self.inputs = inputs;
        self
    }

    pub fn outputs(mut self, outputs: Vec<TransactionOutput>) -> Self {
        self.outputs = outputs;
        self
    }

    pub fn output_type(mut self, output_type: Option<String>) -> Self {
        self.output_type = output_type;
        self
    }

    pub fn coinbase_extra(mut self, extra: Option<String>) -> Self {
        self.coinbase_extra = extra;
        self
    }

    pub fn memo_hex(mut self, hex: Option<String>) -> Self {
        self.memo_hex = hex;
        self
    }

    pub fn sent_output_hashes(mut self, hashes: Vec<FixedHash>) -> Self {
        self.sent_output_hashes = hashes;
        self
    }

    pub fn build(self) -> Result<DisplayedTransaction, ProcessorError> {
        let amount = self
            .amount
            .ok_or_else(|| ProcessorError::ParseError("amount is required".to_string()))?;
        let direction = self
            .direction
            .ok_or_else(|| ProcessorError::ParseError("direction is required".to_string()))?;

        let counterparty = self.counterparty_address.map(|address| CounterpartyInfo {
            address,
            address_emoji: self.counterparty_emoji,
            label: None,
        });

        let fee = match direction {
            TransactionDirection::Outgoing => self.fee.map(|f| FeeInfo {
                amount: f,
                amount_display: format_micro_tari(f),
            }),
            TransactionDirection::Incoming => None,
        };

        let mut payrefs = Vec::new();
        if let Some(block_hash) = &self.block_hash {
            for output_hash in self.sent_output_hashes.iter() {
                let payref = generate_payment_reference(block_hash, output_hash);
                payrefs.push(payref);
            }
        }
        Ok(DisplayedTransaction {
            id: self.id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            direction,
            source: self.source.unwrap_or(TransactionSource::Unknown),
            status: self.status.unwrap_or(TransactionDisplayStatus::Pending),
            amount,
            amount_display: format_micro_tari(amount),
            message: self.message,
            counterparty,
            blockchain: BlockchainInfo {
                block_height: self.block_height.unwrap_or(0),
                timestamp: self.timestamp.unwrap_or_default(),
                confirmations: self.confirmations.unwrap_or(0),
                block_hash: self.block_hash.unwrap_or_default(),
            },
            fee,
            details: TransactionDetails {
                account_id: self.account_id.unwrap_or(0),
                total_credit: self.total_credit,
                total_debit: self.total_debit,
                inputs: self.inputs,
                outputs: self.outputs,
                output_type: self.output_type,
                coinbase_extra: self.coinbase_extra,
                memo_hex: self.memo_hex,
                sent_output_hashes: self.sent_output_hashes,
                sent_payrefs: payrefs,
            },
        })
    }
}
