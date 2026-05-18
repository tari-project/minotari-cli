// Copyright 2026 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

//! Enrich a `DisplayedTransaction` with metadata from a legacy `completed_transactions` row.
//!
//! The outputs table tells us WHAT happened (amount, commitment, status) but not
//! WHO was involved (counterparty address, memo, direction). That metadata lives
//! in `completed_transactions`, which this module grafts onto the output data.

use chrono::{DateTime, NaiveDateTime, Utc};
use tari_common_types::transaction::TxId;
use tari_common_types::types::FixedHash;
use tari_transaction_components::MicroMinotari;
use tari_transaction_components::transaction_components::WalletOutput;

use crate::transactions::{DisplayedTransaction, TransactionDirection, TransactionDisplayStatus, TransactionSource};
use crate::utils::timestamp::current_db_timestamp;

use super::console_db::{ConsoleCompletedTx, STATUS_COMPLETED, STATUS_BROADCAST, STATUS_REJECTED};

/// Build a `DisplayedTransaction` by combining a converted output with legacy
/// transaction metadata.
///
/// # Arguments
///
/// * `output` — The converted `WalletOutput` (from `output_converter`)
/// * `tx` — The matching `ConsoleCompletedTx` from the legacy DB
/// * `height` — The block height at which this output was mined
/// * `is_inbound` — `true` for received outputs, `false` for sent outputs
pub fn enrich_with_transaction_metadata(
    output: &WalletOutput,
    tx: &ConsoleCompletedTx,
    height: u64,
    is_inbound: bool,
) -> DisplayedTransaction {
    // Determine direction
    let direction = if is_inbound {
        TransactionDirection::Inbound
    } else {
        TransactionDirection::Outbound
    };

    // Determine source
    let source = if is_inbound {
        TransactionSource::External
    } else {
        TransactionSource::Internal
    };

    // Determine status
    let status = match tx.status {
        STATUS_COMPLETED => TransactionDisplayStatus::MinedConfirmed,
        STATUS_BROADCAST => TransactionDisplayStatus::Broadcast,
        STATUS_REJECTED => TransactionDisplayStatus::Rejected,
        _ => TransactionDisplayStatus::MinedConfirmed, // default to confirmed
    };

    // Parse timestamp
    let timestamp = DateTime::from_timestamp(tx.timestamp, 0)
        .unwrap_or_else(|| Utc::now());

    // Build the displayed transaction
    DisplayedTransaction {
        id: TxId::new_deterministic(
            // Use a deterministic ID based on tx_id
            &tari_common_types::types::PrivateKey::default(),
            &output.output_hash(),
        ),
        amount: MicroMinotari::from(output.value()),
        fee: MicroMinotari::from(tx.fee as u64),
        direction,
        source,
        status,
        memo: tx.message.clone().unwrap_or_default(),
        blockchain: crate::transactions::BlockchainData {
            height,
            hash: FixedHash::try_from(tx.mined_in_block.clone().unwrap_or_default())
                .unwrap_or(FixedHash::zero()),
            timestamp,
        },
        counterparty: crate::transactions::Counterparty {
            address: if is_inbound {
                tx.source_address.clone()
            } else {
                tx.destination_address.clone()
            },
        },
        payment_reference: None,
        created_at: current_db_timestamp(),
    }
}
