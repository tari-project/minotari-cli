// Copyright 2026 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use anyhow::anyhow;
use tari_common_types::{tari_address::TariAddress, transaction::TxId, types::FixedHash};
use tari_transaction_components::{MicroMinotari, transaction_components::OutputType};

use crate::{
    models::OutputStatus,
    transactions::{
        BlockchainInfo, DisplayedTransaction, FeeInfo, TransactionDetails, TransactionDirection,
        TransactionDisplayStatus, TransactionInput, TransactionOutput, TransactionSource,
    },
};

use super::console_db::{
    ConsoleCompletedTx, STATUS_BROADCAST, STATUS_COINBASE, STATUS_COINBASE_CONFIRMED, STATUS_COINBASE_CONFIRMED_LOCKED,
    STATUS_COINBASE_NOT_IN_BLOCKCHAIN, STATUS_COINBASE_UNCONFIRMED, STATUS_COMPLETED, STATUS_IMPORTED,
    STATUS_MINED_CONFIRMED, STATUS_MINED_CONFIRMED_LOCKED, STATUS_MINED_UNCONFIRMED, STATUS_ONE_SIDED_CONFIRMED,
    STATUS_ONE_SIDED_CONFIRMED_LOCKED, STATUS_ONE_SIDED_UNCONFIRMED, STATUS_QUEUED, STATUS_REJECTED,
};

#[derive(Debug, Clone)]
pub struct ImportedTxInput {
    pub output_hash: FixedHash,
    pub amount: MicroMinotari,
    pub matched_output_id: i64,
    pub mined_in_block_hash: FixedHash,
}

#[derive(Debug, Clone)]
pub struct ImportedTxOutput {
    pub hash: FixedHash,
    pub amount: MicroMinotari,
    pub status: OutputStatus,
    pub mined_in_block_height: u64,
    pub mined_in_block_hash: FixedHash,
    pub output_type: OutputType,
    pub is_change: bool,
}

#[derive(Debug, Clone)]
pub struct TransactionIoSet {
    pub total_credit: MicroMinotari,
    pub total_debit: MicroMinotari,
    pub inputs: Vec<ImportedTxInput>,
    pub outputs: Vec<ImportedTxOutput>,
}

pub fn convert_transaction(
    tx: &ConsoleCompletedTx,
    account_id: i64,
    io: TransactionIoSet,
) -> Result<DisplayedTransaction, anyhow::Error> {
    let direction = map_direction(tx.direction, tx.amount, &io);
    let status = map_display_status(tx.status);
    let source = map_transaction_source(tx.status);
    let amount = MicroMinotari::from(tx.amount.unsigned_abs());
    let fee = MicroMinotari::from(tx.fee.unsigned_abs());
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

    Ok(DisplayedTransaction {
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
            total_credit: io.total_credit,
            total_debit: io.total_debit,
            inputs: io
                .inputs
                .into_iter()
                .map(|input| TransactionInput {
                    output_hash: input.output_hash,
                    amount: input.amount,
                    matched_output_id: input.matched_output_id,
                    mined_in_block_hash: input.mined_in_block_hash,
                })
                .collect(),
            outputs: io
                .outputs
                .into_iter()
                .map(|output| TransactionOutput {
                    hash: output.hash,
                    amount: output.amount,
                    status: output.status,
                    mined_in_block_height: output.mined_in_block_height,
                    mined_in_block_hash: output.mined_in_block_hash,
                    output_type: output.output_type,
                    is_change: output.is_change,
                })
                .collect(),
            output_type: None,
            coinbase_extra: None,
            memo_hex,
            sent_output_hashes: Vec::new(),
            sent_payrefs: Vec::new(),
        },
    })
}

fn map_direction(direction: Option<i64>, amount: i64, io: &TransactionIoSet) -> TransactionDirection {
    match direction {
        Some(1) => TransactionDirection::Outgoing,
        Some(0) => TransactionDirection::Incoming,
        _ if amount < 0 => TransactionDirection::Outgoing,
        _ if io.total_debit > io.total_credit => TransactionDirection::Outgoing,
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
