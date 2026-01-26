use std::collections::{HashMap, HashSet};

use super::error::ProcessorError;
use super::formatting::AMOUNT_MATCHING_TOLERANCE;
use super::resolver::TransactionDataResolver;
use crate::models::{BalanceChange, Id};
use chrono::NaiveDateTime;
use tari_common_types::types::FixedHash;

/// A grouped transaction consisting of one output (credit) and zero or more inputs (debits).
#[derive(Debug, Clone)]
pub struct GroupedTransaction {
    pub output_change: Option<BalanceChange>,
    pub input_changes: Vec<BalanceChange>,
    pub account_id: Id,
    pub effective_height: u64,
    pub effective_date: NaiveDateTime,
    pub sender: Option<String>,
    pub recipient: Option<String>,
    pub memo_parsed: Option<String>,
    pub memo_hex: Option<String>,
    pub claimed_fee: Option<u64>,
}

/// Groups related balance changes into logical transactions.
pub struct BalanceChangeGrouper<'a, R: TransactionDataResolver> {
    resolver: &'a R,
}

impl<'a, R: TransactionDataResolver> BalanceChangeGrouper<'a, R> {
    pub fn new(resolver: &'a R) -> Self {
        Self { resolver }
    }

    pub fn group(
        &self,
        changes: Vec<BalanceChange>,
        input_hash_map: &HashMap<FixedHash, BalanceChange>,
    ) -> Result<Vec<GroupedTransaction>, ProcessorError> {
        let (coinbase_changes, regular_changes) = self.separate_coinbase(changes);

        let mut result: Vec<GroupedTransaction> = Vec::new();

        for coinbase in coinbase_changes {
            result.push(self.coinbase_to_group(coinbase));
        }

        let by_block = self.group_by_block(regular_changes);

        for (base_key, block_changes) in by_block {
            let merged = self.merge_within_block(block_changes, input_hash_map)?;

            for group in merged {
                result.push(GroupedTransaction {
                    output_change: group.output_change,
                    input_changes: group.input_changes,
                    account_id: base_key.account_id,
                    effective_height: base_key.effective_height,
                    effective_date: base_key.effective_date,
                    //hash: base_key.hash,
                    sender: group.sender,
                    recipient: group.recipient,
                    memo_parsed: group.memo_parsed,
                    memo_hex: group.memo_hex,
                    claimed_fee: group.claimed_fee,
                });
            }
        }

        Ok(result)
    }

    fn separate_coinbase(&self, changes: Vec<BalanceChange>) -> (Vec<BalanceChange>, Vec<BalanceChange>) {
        let mut coinbase = Vec::new();
        let mut regular = Vec::new();

        for change in changes {
            if change.balance_credit > 0 && change.description.to_lowercase().contains("coinbase") {
                coinbase.push(change);
            } else {
                regular.push(change);
            }
        }

        (coinbase, regular)
    }

    fn coinbase_to_group(&self, coinbase: BalanceChange) -> GroupedTransaction {
        GroupedTransaction {
            account_id: coinbase.account_id,
            effective_height: coinbase.effective_height,
            effective_date: coinbase.effective_date,
            sender: coinbase.claimed_sender_address.clone(),
            recipient: coinbase.claimed_recipient_address.clone(),
            memo_parsed: coinbase.memo_parsed.clone(),
            memo_hex: coinbase.memo_hex.clone(),
            claimed_fee: coinbase.claimed_fee,
            output_change: Some(coinbase),
            input_changes: vec![],
        }
    }

    fn group_by_block(&self, changes: Vec<BalanceChange>) -> HashMap<BlockKey, Vec<BalanceChange>> {
        let mut groups: HashMap<BlockKey, Vec<BalanceChange>> = HashMap::new();

        for change in changes {
            let key = BlockKey {
                account_id: change.account_id,
                effective_height: change.effective_height,
                effective_date: change.effective_date,
            };
            groups.entry(key).or_default().push(change);
        }

        groups
    }

    fn merge_within_block(
        &self,
        changes: Vec<BalanceChange>,
        input_hash_map: &HashMap<FixedHash, BalanceChange>,
    ) -> Result<Vec<MergedGroup>, ProcessorError> {
        let (outputs, inputs) = self.separate_outputs_inputs(changes);

        if outputs.is_empty() && inputs.is_empty() {
            return Ok(Vec::new());
        }

        if outputs.is_empty() {
            return Ok(vec![self.create_inputs_only_group(inputs)]);
        }

        let mut groups = Vec::new();
        let mut used_input_indices: HashSet<usize> = HashSet::new();

        for output_change in &outputs {
            let (group, matched_indices) =
                self.match_inputs_by_hashes(output_change, &inputs, &used_input_indices, input_hash_map)?;

            used_input_indices.extend(matched_indices);
            groups.push(group);
        }

        let remaining_inputs: Vec<(usize, &BalanceChange)> = inputs
            .iter()
            .enumerate()
            .filter(|(idx, _)| !used_input_indices.contains(idx))
            .collect();

        if !remaining_inputs.is_empty() {
            self.assign_remaining_inputs(&mut groups, remaining_inputs);
        }

        Ok(groups)
    }

    fn separate_outputs_inputs(&self, changes: Vec<BalanceChange>) -> (Vec<BalanceChange>, Vec<BalanceChange>) {
        let mut outputs = Vec::new();
        let mut inputs = Vec::new();

        for change in changes {
            if change.balance_credit > 0 {
                outputs.push(change);
            } else if change.balance_debit > 0 {
                inputs.push(change);
            }
        }

        (outputs, inputs)
    }

    fn create_inputs_only_group(&self, inputs: Vec<BalanceChange>) -> MergedGroup {
        MergedGroup {
            output_change: None,
            input_changes: inputs,
            sender: None,
            recipient: None,
            memo_parsed: None,
            memo_hex: None,
            claimed_fee: None,
        }
    }

    fn match_inputs_by_hashes(
        &self,
        output_change: &BalanceChange,
        inputs: &[BalanceChange],
        used_indices: &HashSet<usize>,
        input_hash_map: &HashMap<FixedHash, BalanceChange>,
    ) -> Result<(MergedGroup, Vec<usize>), ProcessorError> {
        let mut group = MergedGroup {
            output_change: Some(output_change.clone()),
            input_changes: Vec::new(),
            sender: output_change.claimed_sender_address.clone(),
            recipient: output_change.claimed_recipient_address.clone(),
            memo_parsed: output_change.memo_parsed.clone(),
            memo_hex: output_change.memo_hex.clone(),
            claimed_fee: output_change.claimed_fee,
        };

        let sent_hashes = self.resolver.get_sent_output_hashes(output_change)?;
        let mut matched_indices = Vec::new();

        for sent_hash in &sent_hashes {
            if let Some(input_change) = input_hash_map.get(sent_hash) {
                for (idx, input) in inputs.iter().enumerate() {
                    if !used_indices.contains(&idx)
                        && !matched_indices.contains(&idx)
                        && input.balance_debit == input_change.balance_debit
                        && input.effective_height == input_change.effective_height
                    {
                        group.input_changes.push(input.clone());
                        matched_indices.push(idx);
                        break;
                    }
                }
            }
        }

        Ok((group, matched_indices))
    }

    fn assign_remaining_inputs(&self, groups: &mut [MergedGroup], remaining: Vec<(usize, &BalanceChange)>) {
        let non_coinbase_indices: Vec<usize> = groups
            .iter()
            .enumerate()
            .filter(|(_, g)| {
                !g.output_change
                    .as_ref()
                    .map(|c| c.description.to_lowercase().contains("coinbase"))
                    .unwrap_or(false)
            })
            .map(|(i, _)| i)
            .collect();

        if non_coinbase_indices.is_empty() {
            return;
        }

        if non_coinbase_indices.len() == 1 {
            let idx = non_coinbase_indices[0];
            for (_, input) in remaining {
                groups[idx].input_changes.push(input.clone());
            }
            return;
        }

        for (_, input) in remaining {
            let mut assigned = false;

            for &group_idx in &non_coinbase_indices {
                let group = &groups[group_idx];

                if !group.input_changes.is_empty() {
                    continue;
                }

                let claimed_amount = group.output_change.as_ref().and_then(|c| c.claimed_amount);
                let output_credit = group.output_change.as_ref().map(|c| c.balance_credit).unwrap_or(0);

                if let Some(claimed_amount) = claimed_amount {
                    let target_sum = output_credit + claimed_amount;
                    let diff = input.balance_debit.abs_diff(target_sum);

                    if diff <= AMOUNT_MATCHING_TOLERANCE {
                        groups[group_idx].input_changes.push(input.clone());
                        assigned = true;
                        break;
                    }
                }
            }

            if !assigned {
                for &group_idx in &non_coinbase_indices {
                    if groups[group_idx].input_changes.is_empty() {
                        groups[group_idx].input_changes.push(input.clone());
                        assigned = true;
                        break;
                    }
                }
            }

            if !assigned && let Some(&group_idx) = non_coinbase_indices.first() {
                groups[group_idx].input_changes.push(input.clone());
            }
        }
    }
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct BlockKey {
    account_id: Id,
    effective_height: u64,
    // hash: FixedHash,
    effective_date: NaiveDateTime,
}

#[derive(Debug, Clone)]
pub(crate) struct MergedGroup {
    pub output_change: Option<BalanceChange>,
    pub input_changes: Vec<BalanceChange>,
    pub sender: Option<String>,
    pub recipient: Option<String>,
    pub memo_parsed: Option<String>,
    pub memo_hex: Option<String>,
    pub claimed_fee: Option<u64>,
}

/// Build a map from output_hash (hex) to the BalanceChange that represents the input spending it.
pub fn build_input_hash_map<R: TransactionDataResolver>(
    balance_changes: &[BalanceChange],
    resolver: &R,
) -> Result<HashMap<FixedHash, BalanceChange>, ProcessorError> {
    let mut map: HashMap<FixedHash, BalanceChange> = HashMap::new();

    for change in balance_changes {
        if change.balance_debit > 0
            && let Some((output_hash, _mined_hash)) = resolver.get_input_output_hash(change)?
        {
            map.insert(output_hash, change.clone());
        }
    }

    Ok(map)
}
