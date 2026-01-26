use log::debug;
use rusqlite::Connection;
use tari_common_types::types::FixedHash;
use super::builder::DisplayedTransactionBuilder;
use super::error::ProcessorError;
use super::formatting::{address_to_emoji, determine_transaction_source};
use super::grouping::{BalanceChangeGrouper, GroupedTransaction, build_input_hash_map};
use super::resolver::{DatabaseResolver, InMemoryResolver, ProcessingContext, TransactionDataResolver};
use super::types::{
    DisplayedTransaction, TransactionDisplayStatus, TransactionInput, TransactionOutput, TransactionSource,
};
use crate::db::{self, SqlitePool};
use crate::models::{BalanceChange, Id};

/// Processes balance changes into user-displayable transactions.
pub struct DisplayedTransactionProcessor {
    current_tip_height: u64,
}

impl DisplayedTransactionProcessor {
    pub fn new(current_tip_height: u64) -> Self {
        Self { current_tip_height }
    }

    pub fn process_balance_changes(
        &self,
        balance_changes: Vec<BalanceChange>,
        context: ProcessingContext<'_>,
    ) -> Result<Vec<DisplayedTransaction>, ProcessorError> {
        debug!(
            count = balance_changes.len();
            "Processing balance changes"
        );

        match context {
            ProcessingContext::Database(pool) => {
                let resolver = DatabaseResolver::new(pool.clone());
                self.process_with_resolver(balance_changes, &resolver)
            },
            ProcessingContext::InMemory {
                detected_outputs,
                spent_inputs,
            } => {
                let resolver = InMemoryResolver::new(detected_outputs, spent_inputs);
                self.process_with_resolver(balance_changes, &resolver)
            },
        }
    }

    pub fn process_with_resolver<R: TransactionDataResolver>(
        &self,
        balance_changes: Vec<BalanceChange>,
        resolver: &R,
    ) -> Result<Vec<DisplayedTransaction>, ProcessorError> {
        if balance_changes.is_empty() {
            return Ok(Vec::new());
        }

        let input_hash_map = build_input_hash_map(&balance_changes, resolver)?;

        let grouper = BalanceChangeGrouper::new(resolver);
        let groups = grouper.group(balance_changes, &input_hash_map)?;

        let mut transactions = Vec::with_capacity(groups.len());
        for group in groups {
            let tx = self.build_transaction(group, resolver)?;
            transactions.push(tx);
        }

        self.match_inputs(&mut transactions, resolver)?;

        Ok(transactions)
    }

    pub fn process_all_stored(
        &self,
        account_id: Id,
        db_pool: &SqlitePool,
    ) -> Result<Vec<DisplayedTransaction>, ProcessorError> {
        let conn = db_pool.get().map_err(|e| ProcessorError::DbError(e.into()))?;
        self.process_all_stored_with_conn(account_id, &conn, db_pool)
    }

    pub fn process_all_stored_with_conn(
        &self,
        account_id: Id,
        conn: &Connection,
        db_pool: &SqlitePool,
    ) -> Result<Vec<DisplayedTransaction>, ProcessorError> {
        debug!(
            account_id = account_id;
            "Processing all stored transactions for account"
        );

        let balance_changes = db::get_all_balance_changes_by_account_id(conn, account_id)?;

        if balance_changes.is_empty() {
            return Ok(Vec::new());
        }

        let context = ProcessingContext::Database(db_pool);
        let mut transactions = self.process_balance_changes(balance_changes, context)?;

        transactions.sort_by(|a, b| b.blockchain.timestamp.cmp(&a.blockchain.timestamp));

        Ok(transactions)
    }

    fn build_transaction<R: TransactionDataResolver>(
        &self,
        group: GroupedTransaction,
        resolver: &R,
    ) -> Result<DisplayedTransaction, ProcessorError> {
        let total_credit = group.output_change.as_ref().map(|c| c.balance_credit).unwrap_or(0);
        let total_debit: u64 = group.input_changes.iter().map(|c| c.balance_debit).sum();

        let (outputs, output_type_str, coinbase_extra, is_coinbase) = self.collect_output_details(&group, resolver)?;

        let inputs = self.collect_input_details(&group, resolver)?;

        let mined_hash = match (outputs.first(), inputs.first()) {
            (Some(output), _) => output.mined_in_block_hash,
            (_, Some(input)) => input.mined_in_block_hash,
            _ => FixedHash::default() //this should not happen as the display tx should have at least one input or output,
        };

        let source = self.determine_source(&group, is_coinbase);
        let status = self.determine_status(group.effective_height);
        let (counterparty_addr, counterparty_emoji) = self.determine_counterparty(&group, total_credit, total_debit);

        let confirmations = self.current_tip_height.saturating_sub(group.effective_height);

        DisplayedTransactionBuilder::new()
            .account_id(group.account_id)
            .source(source)
            .status(status)
            .credits_and_debits(total_credit, total_debit)
            .message(group.memo_parsed)
            .counterparty(counterparty_addr, counterparty_emoji)
            .blockchain_info(group.effective_height, mined_hash, group.effective_date, confirmations)
            .fee(group.claimed_fee)
            .inputs(inputs)
            .outputs(outputs)
            .output_type(output_type_str)
            .coinbase_extra(coinbase_extra)
            .memo_hex(group.memo_hex)
            .build()
    }

    #[allow(clippy::type_complexity)]
    fn collect_output_details<R: TransactionDataResolver>(
        &self,
        group: &GroupedTransaction,
        resolver: &R,
    ) -> Result<(Vec<TransactionOutput>, Option<String>, Option<String>, bool), ProcessorError> {
        let mut outputs = Vec::new();
        let mut output_type_str: Option<String> = None;
        let mut coinbase_extra: Option<String> = None;
        let mut is_coinbase = false;

        if let Some(ref output_change) = group.output_change
            && let Some(details) = resolver.get_output_details(output_change)?
        {
            is_coinbase = details.is_coinbase;
            output_type_str = Some(details.output_type.clone());
            coinbase_extra = details.coinbase_extra.clone();

            outputs.push(TransactionOutput {
                hash: details.hash,
                amount: output_change.balance_credit,
                status: details.status,
                mined_in_block_height: details.mined_in_block_height,
                mined_in_block_hash: details.mined_hash,
                output_type: details.output_type,
                is_change: false,
            });
        }

        Ok((outputs, output_type_str, coinbase_extra, is_coinbase))
    }

    fn collect_input_details<R: TransactionDataResolver>(
        &self,
        group: &GroupedTransaction,
        resolver: &R,
    ) -> Result<Vec<TransactionInput>, ProcessorError> {
        let mut inputs = Vec::new();

        for input_change in &group.input_changes {
            if let Some((output_hash, mined_hash)) = resolver.get_input_output_hash(input_change)? {
                inputs.push(TransactionInput {
                    output_hash,
                    amount: input_change.balance_debit,
                    mined_in_block_hash: mined_hash,
                    matched_output_id: None,
                    is_matched: false,
                });
            }
        }

        Ok(inputs)
    }

    fn determine_source(&self, group: &GroupedTransaction, is_coinbase: bool) -> TransactionSource {
        let has_sender = group.sender.is_some();
        let has_recipient = group.recipient.is_some();
        determine_transaction_source(is_coinbase, has_sender, has_recipient)
    }

    fn determine_status(&self, effective_height: u64) -> TransactionDisplayStatus {
        let confirmations = self.current_tip_height.saturating_sub(effective_height);
        if confirmations > 0 {
            TransactionDisplayStatus::Confirmed
        } else {
            TransactionDisplayStatus::Pending
        }
    }

    fn determine_counterparty(
        &self,
        group: &GroupedTransaction,
        total_credit: u64,
        total_debit: u64,
    ) -> (Option<String>, Option<String>) {
        if total_debit > total_credit {
            (
                group.recipient.clone(),
                group.recipient.as_ref().and_then(|a| address_to_emoji(a)),
            )
        } else {
            (
                group.sender.clone(),
                group.sender.as_ref().and_then(|a| address_to_emoji(a)),
            )
        }
    }

    fn match_inputs<R: TransactionDataResolver>(
        &self,
        transactions: &mut [DisplayedTransaction],
        resolver: &R,
    ) -> Result<(), ProcessorError> {
        let output_map = resolver.build_output_hash_map()?;

        for tx in transactions.iter_mut() {
            for input in &mut tx.details.inputs {
                if let Some(&output_id) = output_map.get(&input.output_hash) {
                    input.matched_output_id = Some(output_id);
                    input.is_matched = true;
                }
            }
        }

        Ok(())
    }
}
