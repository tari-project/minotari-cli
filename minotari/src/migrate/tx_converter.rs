// Copyright 2026 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use anyhow::anyhow;
use tari_common_types::{tari_address::TariAddress, transaction::TxId, types::FixedHash};
use tari_transaction_components::MicroMinotari;

use crate::{
    models::BalanceChange,
    transactions::{
        BlockchainInfo, DisplayedTransaction, FeeInfo, TransactionDetails, TransactionDirection,
        TransactionDisplayStatus, TransactionSource,
    },
};

use super::console_db::{
    ConsoleCompletedTx, STATUS_BROADCAST, STATUS_COINBASE, STATUS_COINBASE_CONFIRMED, STATUS_COINBASE_CONFIRMED_LOCKED,
    STATUS_COINBASE_NOT_IN_BLOCKCHAIN, STATUS_COINBASE_UNCONFIRMED, STATUS_COMPLETED, STATUS_IMPORTED,
    STATUS_MINED_CONFIRMED, STATUS_MINED_CONFIRMED_LOCKED, STATUS_MINED_UNCONFIRMED, STATUS_ONE_SIDED_CONFIRMED,
    STATUS_ONE_SIDED_CONFIRMED_LOCKED, STATUS_ONE_SIDED_UNCONFIRMED, STATUS_QUEUED, STATUS_REJECTED,
};

#[derive(Debug, Clone)]
pub struct ConvertedTransaction {
    pub displayed: DisplayedTransaction,
    pub metadata_balance_change: BalanceChange,
}

pub fn convert_transaction(tx: &ConsoleCompletedTx, account_id: i64) -> Result<ConvertedTransaction, anyhow::Error> {
    let direction = map_direction(tx.direction, tx.amount);
    let status = map_display_status(tx.status);
    let source = map_transaction_source(tx.status);
    let amount = MicroMinotari::from(tx.amount.max(0) as u64);
    let fee = MicroMinotari::from(tx.fee.max(0) as u64);
    let block_hash = tx
        .mined_in_block
        .clone()
        .map(FixedHash::try_from)
        .transpose()
        .map_err(|_| anyhow!("Invalid mined_in_block hash for tx {}", tx.tx_id))?
        .unwrap_or_default();
    let counterparty = match direction {
        TransactionDirection::Incoming => parse_address(&tx.source_address),
        TransactionDirection::Outgoing => parse_address(&tx.destination_address),
    };
    let memo_hex = tx.user_payment_id.as_ref().or(tx.payment_id.as_ref()).map(hex::encode);

    let displayed = DisplayedTransaction {
        id: TxId::from(tx.tx_id as u64),
        direction,
        source,
        status,
        amount,
        message: None,
        counterparty,
        blockchain: BlockchainInfo {
            block_height: tx.mined_height.unwrap_or_default() as u64,
            timestamp: tx.timestamp,
            confirmations: initial_confirmations(status),
            block_hash,
        },
        fee: if matches!(direction, TransactionDirection::Outgoing) {
            Some(FeeInfo { amount: fee })
        } else {
            None
        },
        details: TransactionDetails {
            account_id,
            total_credit: if matches!(direction, TransactionDirection::Incoming) {
                amount
            } else {
                MicroMinotari::from(0)
            },
            total_debit: if matches!(direction, TransactionDirection::Outgoing) {
                amount.saturating_add(fee)
            } else {
                MicroMinotari::from(0)
            },
            inputs: Vec::new(),
            outputs: Vec::new(),
            output_type: None,
            coinbase_extra: None,
            memo_hex: memo_hex.clone(),
            sent_output_hashes: Vec::new(),
            sent_payrefs: Vec::new(),
        },
    };

    let metadata_balance_change = BalanceChange {
        account_id,
        caused_by_output_id: None,
        caused_by_input_id: None,
        description: format!("Migrated console wallet transaction {}", tx.tx_id),
        balance_credit: MicroMinotari::from(0),
        balance_debit: MicroMinotari::from(0),
        effective_date: tx.timestamp,
        effective_height: tx.mined_height.unwrap_or_default() as u64,
        claimed_recipient_address: parse_address(&tx.destination_address),
        claimed_sender_address: parse_address(&tx.source_address),
        memo_parsed: None,
        memo_hex,
        claimed_fee: Some(fee),
        claimed_amount: Some(amount),
        is_reversal: false,
        reversal_of_balance_change_id: None,
        is_reversed: false,
    };

    Ok(ConvertedTransaction {
        displayed,
        metadata_balance_change,
    })
}

fn map_direction(direction: Option<i64>, amount: i64) -> TransactionDirection {
    match direction {
        Some(1) => TransactionDirection::Outgoing,
        Some(0) => TransactionDirection::Incoming,
        _ if amount < 0 => TransactionDirection::Outgoing,
        _ => TransactionDirection::Incoming,
    }
}

fn map_display_status(status: i64) -> TransactionDisplayStatus {
    match status {
        STATUS_REJECTED => TransactionDisplayStatus::Rejected,
        STATUS_MINED_UNCONFIRMED | STATUS_ONE_SIDED_UNCONFIRMED | STATUS_COINBASE_UNCONFIRMED => {
            TransactionDisplayStatus::Unconfirmed
        },
        STATUS_MINED_CONFIRMED
        | STATUS_MINED_CONFIRMED_LOCKED
        | STATUS_ONE_SIDED_CONFIRMED
        | STATUS_ONE_SIDED_CONFIRMED_LOCKED
        | STATUS_COINBASE
        | STATUS_COINBASE_CONFIRMED
        | STATUS_COINBASE_CONFIRMED_LOCKED
        | STATUS_COINBASE_NOT_IN_BLOCKCHAIN
        | STATUS_IMPORTED => TransactionDisplayStatus::Confirmed,
        STATUS_COMPLETED | STATUS_BROADCAST | STATUS_QUEUED => TransactionDisplayStatus::Pending,
        _ => TransactionDisplayStatus::Pending,
    }
}

fn map_transaction_source(status: i64) -> TransactionSource {
    match status {
        STATUS_COINBASE
        | STATUS_COINBASE_UNCONFIRMED
        | STATUS_COINBASE_CONFIRMED
        | STATUS_COINBASE_NOT_IN_BLOCKCHAIN
        | STATUS_COINBASE_CONFIRMED_LOCKED => TransactionSource::Coinbase,
        STATUS_ONE_SIDED_UNCONFIRMED | STATUS_ONE_SIDED_CONFIRMED | STATUS_ONE_SIDED_CONFIRMED_LOCKED => {
            TransactionSource::OneSided
        },
        _ => TransactionSource::Transfer,
    }
}

fn initial_confirmations(status: TransactionDisplayStatus) -> u64 {
    match status {
        TransactionDisplayStatus::Confirmed => 3,
        TransactionDisplayStatus::Unconfirmed => 1,
        _ => 0,
    }
}

fn parse_address(bytes: &[u8]) -> Option<TariAddress> {
    TariAddress::from_bytes(bytes).ok()
}
