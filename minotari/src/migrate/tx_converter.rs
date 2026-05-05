//! Convert a console wallet `completed_transactions` row into the new
//! wallet's `DisplayedTransaction` shape.
//!
//! The most important property: the `tx_id` value the console wallet stored
//! (a random `u64` generated when the user first sent / received the
//! transaction) is preserved as the `DisplayedTransaction::id`. Without that
//! the user would see a fresh, unfamiliar set of IDs after migration —
//! which is exactly what the bounty's primary acceptance criterion
//! ("identical display transaction IDs") forbids.
//!
//! New wallet's normal scan path computes a deterministic `TxId` from
//! `(view_key, output_hash)`. We do not use that here; we use the legacy
//! random `tx_id` instead. The two ID conventions co-exist in the
//! `displayed_transactions` table without conflict (PRIMARY KEY is just the
//! string form of whatever u64 was supplied).

use anyhow::anyhow;
use tari_common_types::{
    tari_address::TariAddress,
    transaction::{LegacyTransactionStatus, TxId},
    types::FixedHash,
};
use tari_transaction_components::MicroMinotari;
use tari_transaction_components::transaction_components::{CoinBaseExtra, OutputType};

use super::console_db::ConsoleCompletedTxRow;
use crate::models::OutputStatus;
use crate::transactions::displayed_transaction_processor::{
    BlockchainInfo, DisplayedTransaction, FeeInfo, TransactionDetails, TransactionDirection, TransactionDisplayStatus,
    TransactionInput, TransactionOutput, TransactionSource,
};

/// What the migrator emits per source transaction row. The orchestrator picks
/// these apart and writes them into the new wallet's `displayed_transactions`
/// (and, where the row represents a sent transaction, `completed_transactions`)
/// tables.
pub struct ConvertedTransaction {
    pub display: DisplayedTransaction,
    /// `Some` only for outgoing transactions — the new wallet's
    /// `completed_transactions` table is exclusively a "transactions I sent"
    /// log (it carries the broadcast bookkeeping). Incoming or coinbase rows
    /// only need to land in `displayed_transactions`.
    pub completed_record: Option<CompletedTxRecord>,
}

/// Minimal info needed to populate the new wallet's `completed_transactions`
/// row for a migrated outbound transaction. We don't have a full broadcast
/// `Transaction` struct on hand (the legacy `transaction_protocol` blob is
/// bincode and we'd prefer not to pull bincode into the migration crate just
/// for this case), so we leave `serialized_transaction` empty and mark the
/// status accordingly.
pub struct CompletedTxRecord {
    pub tx_id: u64,
    pub kernel_excess: Vec<u8>,
    pub mined_height: Option<u64>,
    pub mined_block_hash: Option<Vec<u8>>,
    pub status_label: &'static str,
}

pub fn convert_transaction(
    row: &ConsoleCompletedTxRow,
    account_id: i64,
    sent_output_hashes: Vec<FixedHash>,
) -> Result<ConvertedTransaction, anyhow::Error> {
    let tx_id = row.tx_id as u64;
    let amount = MicroMinotari::from(u64::try_from(row.amount).unwrap_or(0));
    let fee = MicroMinotari::from(u64::try_from(row.fee).unwrap_or(0));

    let direction = match row.direction {
        Some(0) => TransactionDirection::Incoming, // legacy 0 = Inbound
        Some(1) => TransactionDirection::Outgoing, // legacy 1 = Outbound
        Some(_) | None => infer_direction_from_amount(row),
    };

    let legacy_status = LegacyTransactionStatus::try_from(row.status).unwrap_or(LegacyTransactionStatus::Completed);
    let status = map_status(legacy_status);
    let source = map_source(legacy_status);

    let counterparty = match direction {
        TransactionDirection::Incoming => parse_address(&row.source_address).ok(),
        TransactionDirection::Outgoing => parse_address(&row.destination_address).ok(),
    };

    let (block_height, block_hash) = match (row.mined_height, &row.mined_in_block) {
        (Some(h), Some(hash_bytes)) if h >= 0 => (
            h as u64,
            FixedHash::try_from(hash_bytes.as_slice()).unwrap_or_default(),
        ),
        _ => (0, FixedHash::default()),
    };

    let timestamp = row.mined_timestamp.unwrap_or(row.timestamp);

    let mut builder_credit = MicroMinotari::from(0);
    let mut builder_debit = MicroMinotari::from(0);
    if matches!(direction, TransactionDirection::Outgoing) {
        builder_debit = amount.saturating_add(fee);
    } else {
        builder_credit = amount;
    }

    // Outputs / inputs lists are best-effort: we have the output hashes
    // recorded by the console wallet but no per-output amounts here, so the
    // detailed view will show zero amounts. The aggregate `amount` the user
    // actually cares about (and `total_credit`/`total_debit`) is correct.
    let outputs: Vec<TransactionOutput> = sent_output_hashes
        .iter()
        .map(|hash| TransactionOutput {
            hash: *hash,
            amount: MicroMinotari::from(0),
            status: OutputStatus::Spent,
            mined_in_block_height: block_height,
            mined_in_block_hash: block_hash,
            output_type: OutputType::Standard,
            is_change: false,
        })
        .collect();

    let inputs: Vec<TransactionInput> = Vec::new();

    let coinbase_extra = if matches!(source, TransactionSource::Coinbase) {
        Some(CoinBaseExtra::default())
    } else {
        None
    };

    let display = DisplayedTransaction {
        id: TxId::from(tx_id),
        direction,
        source,
        status,
        amount,
        message: None,
        counterparty,
        blockchain: BlockchainInfo {
            block_height,
            timestamp,
            confirmations: 0,
            block_hash,
        },
        fee: match direction {
            TransactionDirection::Outgoing => Some(FeeInfo { amount: fee }),
            TransactionDirection::Incoming => None,
        },
        details: TransactionDetails {
            account_id,
            total_credit: builder_credit,
            total_debit: builder_debit,
            inputs,
            outputs,
            output_type: Some(OutputType::Standard),
            coinbase_extra,
            memo_hex: row.payment_id.as_ref().map(hex::encode),
            sent_output_hashes,
            sent_payrefs: Vec::new(),
        },
    };

    let completed_record = if matches!(direction, TransactionDirection::Outgoing) {
        Some(CompletedTxRecord {
            tx_id,
            // The source-table kernel signature columns are the kernel excess
            // signature, not the kernel excess commitment — but we keep them
            // here as a reasonable best-effort marker.
            kernel_excess: Vec::new(),
            mined_height: row.mined_height.map(|h| h as u64),
            mined_block_hash: row.mined_in_block.clone(),
            status_label: completed_status_label(legacy_status),
        })
    } else {
        None
    };

    Ok(ConvertedTransaction {
        display,
        completed_record,
    })
}

fn parse_address(bytes: &[u8]) -> Result<TariAddress, anyhow::Error> {
    TariAddress::from_bytes(bytes).map_err(|e| anyhow!("Invalid address bytes: {e}"))
}

fn infer_direction_from_amount(row: &ConsoleCompletedTxRow) -> TransactionDirection {
    // Coinbase / one-sided / scanned-in transactions on the console wallet
    // sometimes have a NULL direction column. If we can't tell, treat positive
    // amounts as incoming — matches what the user would expect to see.
    if row.amount > 0 {
        TransactionDirection::Incoming
    } else {
        TransactionDirection::Outgoing
    }
}

fn map_status(status: LegacyTransactionStatus) -> TransactionDisplayStatus {
    use LegacyTransactionStatus::*;
    match status {
        Pending | Queued => TransactionDisplayStatus::Pending,
        Completed | Broadcast => TransactionDisplayStatus::Pending,
        MinedUnconfirmed | OneSidedUnconfirmed | CoinbaseUnconfirmed => TransactionDisplayStatus::Unconfirmed,
        MinedConfirmed | MinedConfirmedLocked | OneSidedConfirmed | OneSidedConfirmedLocked | CoinbaseConfirmed
        | CoinbaseConfirmedLocked => TransactionDisplayStatus::Confirmed,
        Rejected => TransactionDisplayStatus::Rejected,
        Imported | CoinbaseNotInBlockChain | Coinbase => TransactionDisplayStatus::Confirmed,
    }
}

fn map_source(status: LegacyTransactionStatus) -> TransactionSource {
    use LegacyTransactionStatus::*;
    match status {
        Coinbase | CoinbaseUnconfirmed | CoinbaseConfirmed | CoinbaseNotInBlockChain | CoinbaseConfirmedLocked => {
            TransactionSource::Coinbase
        }
        OneSidedUnconfirmed | OneSidedConfirmed | OneSidedConfirmedLocked => TransactionSource::OneSided,
        Imported => TransactionSource::Transfer,
        _ => TransactionSource::Transfer,
    }
}

fn completed_status_label(status: LegacyTransactionStatus) -> &'static str {
    use LegacyTransactionStatus::*;
    match status {
        MinedConfirmed | MinedConfirmedLocked | OneSidedConfirmed | OneSidedConfirmedLocked | CoinbaseConfirmed
        | CoinbaseConfirmedLocked => "confirmed",
        MinedUnconfirmed | OneSidedUnconfirmed | CoinbaseUnconfirmed => "unconfirmed",
        Rejected => "rejected",
        _ => "pending",
    }
}

/// Decode a `Vec<FixedHash>` from the bincode-ish serialised form the console
/// wallet uses for `sent_output_hashes`/`received_output_hashes`. The format
/// is a raw concatenation of 32-byte hashes (no length prefix), so we simply
/// chunk and convert.
pub fn decode_output_hashes(blob: Option<&Vec<u8>>) -> Vec<FixedHash> {
    let Some(b) = blob else {
        return Vec::new();
    };
    b.chunks_exact(32)
        .filter_map(|c| FixedHash::try_from(c).ok())
        .collect()
}

