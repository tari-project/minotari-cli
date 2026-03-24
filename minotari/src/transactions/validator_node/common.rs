//! Shared helpers for validator node pay-to-self transaction construction.
//!
//! All three VN operations (registration, exit, eviction) lock the consensus-required
//! minimum deposit and build a single pay-to-self output with operation-specific
//! [`OutputFeatures`]. This module extracts that common pattern.

use log::info;
use tari_common::configuration::Network;
use tari_common_types::transaction::TxId;
use tari_transaction_components::{
    TransactionBuilder,
    consensus::ConsensusConstantsBuilder,
    offline_signing::{
        models::{PaymentRecipient, PrepareOneSidedTransactionForSigningResult},
        prepare_one_sided_transaction_for_signing,
    },
    tari_amount::MicroMinotari,
    transaction_components::{MemoField, OutputFeatures, memo_field::TxType},
};

use crate::{
    db::{AccountRow, SqlitePool},
    transactions::fund_locker::FundLocker,
};

/// Locks the VN registration deposit and prepares a pay-to-self transaction for signing.
///
/// Used by all three VN operations (registration, exit, eviction) which share the same
/// transaction shape: one output sent back to the sender, carrying operation-specific
/// `output_features`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_vn_pay_to_self_tx(
    account: &AccountRow,
    db_pool: SqlitePool,
    network: Network,
    password: &str,
    output_features: OutputFeatures,
    fee_per_gram: MicroMinotari,
    payment_id: Option<&str>,
    idempotency_key: Option<String>,
    seconds_to_lock: u64,
    confirmation_window: u64,
    tx_description: &str,
) -> Result<PrepareOneSidedTransactionForSigningResult, anyhow::Error> {
    let consensus_constants = ConsensusConstantsBuilder::new(network).build();
    let deposit_amount = consensus_constants.validator_node_registration_min_deposit_amount();

    info!(
        target: "audit",
        deposit_amount = deposit_amount.as_u64();
        "Creating {} transaction", tx_description
    );

    let sender_address = account.get_address(network, password)?;
    let fund_locker = FundLocker::new(db_pool);
    let locked_funds = fund_locker.lock(
        account.id,
        deposit_amount,
        1,
        fee_per_gram,
        None,
        idempotency_key,
        seconds_to_lock,
        confirmation_window,
    )?;

    let key_manager = account.get_key_manager(password)?;
    let mut tx_builder = TransactionBuilder::new(consensus_constants, key_manager, network)?;
    tx_builder.with_fee_per_gram(fee_per_gram);
    for utxo in &locked_funds.utxos {
        tx_builder.with_input(utxo.clone())?;
    }

    let tx_id = TxId::new_random();
    let memo = payment_id
        .and_then(|s| MemoField::new_open_from_string(s, TxType::PaymentToOther).ok())
        .unwrap_or_default();

    let payment_recipient = PaymentRecipient {
        amount: deposit_amount,
        output_features,
        address: sender_address.clone(),
        payment_id: memo.clone(),
    };

    prepare_one_sided_transaction_for_signing(tx_id, tx_builder, &[payment_recipient], memo, sender_address)
        .map_err(Into::into)
}
