//! Shared helpers for validator node pay-to-self transaction construction.
//!
//! Both VN operations (registration and exit) lock the consensus-required
//! minimum deposit and build a single pay-to-self output with operation-specific
//! [`OutputFeatures`]. This module extracts that common pattern.

use anyhow::anyhow;
use log::info;
use rusqlite::Connection;
use tari_common::configuration::Network;
use tari_common_types::{transaction::TxId, types::FixedHash};
use tari_script::script;
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
    db::AccountRow,
    transactions::{
        fee_estimator::{estimated_output_size_for_send, measure_output_size},
        fund_locker::{FundLocker, release_reservation},
        idempotency::{IdempotencyBinding, IdempotencyOperation, RequestFingerprint},
        input_selector::map_change_required_error,
    },
};

/// Locks the VN registration deposit and prepares a pay-to-self transaction for signing.
///
/// Used by both VN operations (registration and exit) which share the same
/// transaction shape: one output sent back to the sender, carrying operation-specific
/// `output_features`.
///
/// `operation` both names the transaction in the audit log and scopes the
/// idempotency key, so a registration key cannot be redeemed for an exit.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_vn_pay_to_self_tx(
    account: &AccountRow,
    conn: &mut Connection,
    network: Network,
    password: &str,
    output_features: OutputFeatures,
    fee_per_gram: MicroMinotari,
    payment_id: Option<&str>,
    idempotency_key: Option<String>,
    seconds_to_lock: u64,
    confirmation_window: u64,
    operation: IdempotencyOperation,
) -> Result<PrepareOneSidedTransactionForSigningResult, anyhow::Error> {
    let consensus_constants = ConsensusConstantsBuilder::new(network).build();
    let deposit_amount = consensus_constants.validator_node_registration_min_deposit_amount();

    info!(
        target: "audit",
        deposit_amount = deposit_amount.as_u64();
        "Creating {} transaction", operation.description()
    );

    // `output_features` carries everything that distinguishes one VN operation
    // from another — the node public key, its signature, the claim key, the max
    // epoch — so hashing its canonical encoding binds the
    // key to the specific node action being requested.
    let encoded_features = serde_json::to_vec(&output_features)
        .map_err(|e| anyhow!("Failed to encode validator node output features: {}", e))?;
    let idempotency = IdempotencyBinding::new(
        idempotency_key,
        operation,
        RequestFingerprint::new(operation)
            .field("account_id", account.id.to_le_bytes())
            .field("deposit_amount", deposit_amount.as_u64().to_le_bytes())
            .field("output_features", encoded_features)
            .field("fee_per_gram", fee_per_gram.as_u64().to_le_bytes())
            .optional_field("payment_id", payment_id)
            .field("seconds_to_lock", seconds_to_lock.to_le_bytes())
            .field("confirmation_window", confirmation_window.to_le_bytes()),
    );

    let sender_address = account.get_address(network, password)?;
    let memo = match payment_id {
        Some(s) => {
            let open_memo = MemoField::new_open_from_string(s, TxType::PaymentToOther)
                .map_err(|e| anyhow!("Invalid payment_id: {e}"))?;
            MemoField::new_transaction_info(
                sender_address.clone(),
                MicroMinotari::zero(),
                MicroMinotari::zero(),
                true,
                TxType::PaymentToOther,
                vec![FixedHash::zero()],
                s.as_bytes().to_vec(),
            )
            .map_err(|e| anyhow!("Payment id does not fit in the change memo: {e}"))?;
            open_memo
        },
        None => MemoField::default(),
    };

    let script = script!(PushPubKey(Box::default())).map_err(|e| anyhow!("Failed to build the default script: {e}"))?;
    let vn_output_size = measure_output_size(&output_features, &script, &memo)?;
    let change_output_size = estimated_output_size_for_send(payment_id.map_or(0, str::len), 1, &sender_address)?;
    let estimated_output_size = vn_output_size.max(change_output_size);

    let fund_locker = FundLocker::new();
    let locked_funds = fund_locker.lock(
        conn,
        account.id,
        deposit_amount,
        1,
        fee_per_gram,
        Some(estimated_output_size),
        idempotency,
        seconds_to_lock,
        confirmation_window,
    )?;

    let created_reservation = locked_funds.created_reservation;
    let reservation_id = locked_funds.reservation_id.clone();

    let prepared = (|| -> Result<PrepareOneSidedTransactionForSigningResult, anyhow::Error> {
        let key_manager = account.get_key_manager(password)?;
        let mut tx_builder = TransactionBuilder::new(consensus_constants, key_manager.clone(), network)?;
        tx_builder.with_fee_per_gram(fee_per_gram);
        // Every send carries a change output, so the wallet's scan can see it land.
        tx_builder.with_change_required();
        for utxo in &locked_funds.utxos {
            tx_builder.with_input(utxo.clone())?;
        }

        let tx_id = TxId::new_random();
        let payment_recipient = PaymentRecipient {
            amount: deposit_amount,
            output_features,
            address: sender_address.clone(),
            payment_id: memo.clone(),
        };

        prepare_one_sided_transaction_for_signing(
            &key_manager,
            tx_id,
            tx_builder,
            &[payment_recipient],
            memo,
            sender_address,
        )
        .map_err(map_change_required_error)
    })();

    if prepared.is_err() && created_reservation {
        release_reservation(conn, &reservation_id);
    }

    prepared
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{self, create_account, get_account_by_name, init_db};
    use chrono::Utc;
    use rusqlite::named_params;
    use tari_common_types::{
        epoch::VnEpoch,
        seeds::cipher_seed::CipherSeed,
        types::{CompressedPublicKey, PrivateKey},
    };
    use tari_transaction_components::{
        fee::Fee,
        key_manager::wallet_types::{SeedWordsWallet, WalletType},
        test_helpers::{TestParams, create_wallet_output_with_data},
        transaction_components::ValidatorNodeSignature,
        weight::TransactionWeight,
    };
    use tempfile::tempdir;

    fn sample_vn_registration_features(network: Network) -> OutputFeatures {
        let vn_sk = PrivateKey::from(11u64);
        let claim_sk = PrivateKey::from(22u64);
        let claim_pk = CompressedPublicKey::from_secret_key(&claim_sk);
        let max_epoch = VnEpoch(100);
        let vn_sig =
            ValidatorNodeSignature::sign_for_registration(&vn_sk, network.as_byte(), None, &claim_pk, max_epoch);
        OutputFeatures::for_validator_node_registration(vn_sig, claim_pk, None, max_epoch)
    }

    fn insert_spendable_utxo(conn: &Connection, account: &AccountRow, value: MicroMinotari, seed_byte: u8) {
        let key_manager = account.get_key_manager("pass").unwrap();
        let output = create_wallet_output_with_data(
            script!(Nop).unwrap(),
            OutputFeatures::default(),
            &TestParams::new(&key_manager),
            value,
            &key_manager,
        )
        .unwrap();
        let block_hash = [seed_byte; 32];
        conn.execute(
            r#"
            INSERT INTO outputs (
                account_id, tx_id, output_hash, mined_in_block_height, mined_in_block_hash, value,
                mined_timestamp, wallet_output_json, status, confirmed_height, confirmed_hash, is_burn, maturity
            ) VALUES (:account_id, :tx_id, :hash, 10, :block_hash, :val, :ts, :json, 'UNSPENT', 10, :block_hash, 0, 0)
            "#,
            named_params! {
                ":account_id": account.id,
                ":tx_id": i64::from(seed_byte),
                ":hash": output.output_hash().to_vec(),
                ":block_hash": block_hash.as_slice(),
                ":val": value.as_u64() as i64,
                ":ts": Utc::now(),
                ":json": serde_json::to_string(&output).unwrap(),
            },
        )
        .unwrap();
    }

    #[test]
    fn oversized_payment_id_is_rejected_before_locking_funds() {
        let temp = tempdir().unwrap();
        let pool = init_db(temp.path().join("vn_memo.db")).unwrap();
        let mut conn = pool.get().unwrap();

        let wallet = WalletType::SeedWords(SeedWordsWallet::construct_new(CipherSeed::random()).unwrap());
        create_account(&conn, "vn", &wallet, "pass").unwrap();
        let account = get_account_by_name(&conn, "vn").unwrap().unwrap();
        db::insert_scanned_tip_block(&conn, account.id, 200, &[1u8; 32]).unwrap();

        let consensus = ConsensusConstantsBuilder::new(Network::LocalNet).build();
        let deposit = consensus.validator_node_registration_min_deposit_amount();
        insert_spendable_utxo(&conn, &account, deposit + MicroMinotari(1_000_000), 1);

        // 220 bytes fits in an Open memo (<=254 bytes) but overflows the change TransactionInfo memo.
        let oversized_payment_id = "x".repeat(220);
        let err = build_vn_pay_to_self_tx(
            &account,
            &mut conn,
            Network::LocalNet,
            "pass",
            sample_vn_registration_features(Network::LocalNet),
            MicroMinotari(5),
            Some(&oversized_payment_id),
            Some("vn-memo-key".to_string()),
            3600,
            10,
            IdempotencyOperation::ValidatorNodeRegistration,
        )
        .expect_err("oversized payment_id must be rejected");
        assert!(err.to_string().contains("Payment id"), "unexpected error: {err}");

        let locked_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM outputs WHERE status = 'LOCKED'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(locked_count, 0, "no UTXOs should be locked when payment_id is invalid");
    }

    #[test]
    fn vn_output_features_size_is_measured_during_utxo_selection() {
        let temp = tempdir().unwrap();
        let pool = init_db(temp.path().join("vn_size.db")).unwrap();
        let mut conn = pool.get().unwrap();

        let wallet = WalletType::SeedWords(SeedWordsWallet::construct_new(CipherSeed::random()).unwrap());
        create_account(&conn, "vn", &wallet, "pass").unwrap();
        let account = get_account_by_name(&conn, "vn").unwrap().unwrap();
        db::insert_scanned_tip_block(&conn, account.id, 200, &[1u8; 32]).unwrap();

        let network = Network::LocalNet;
        let consensus = ConsensusConstantsBuilder::new(network).build();
        let deposit = consensus.validator_node_registration_min_deposit_amount();
        let fee_per_gram = MicroMinotari(20);
        let features = sample_vn_registration_features(network);

        let sender_address = account.get_address(network, "pass").unwrap();
        let generic_size = estimated_output_size_for_send(0, 1, &sender_address).unwrap();
        let script = script!(PushPubKey(Box::default())).unwrap();
        let vn_size = measure_output_size(&features, &script, &MemoField::default()).unwrap();
        assert!(
            vn_size > generic_size,
            "VN registration features ({vn_size}) must be larger than generic output features ({generic_size})"
        );

        let fee_calc = Fee::new(TransactionWeight::latest());
        let input_fee = fee_calc.calculate(fee_per_gram, 0, 1, 0, 0);
        let min_change = input_fee.max(MicroMinotari(1));
        let generic_fee = fee_calc.calculate(fee_per_gram, 1, 1, 2, generic_size * 2);
        let real_vn_fee = fee_calc.calculate(fee_per_gram, 1, 1, 2, vn_size * 2);
        assert!(real_vn_fee > generic_fee);

        // Give the wallet one UTXO that covers `deposit + generic_fee + min_change` (so unmeasured
        // locking would pick only this UTXO and then fail at build time) plus a second small UTXO
        // that covers the extra VN features fee + second input fee.
        let utxo_1 = deposit + generic_fee + min_change;
        let utxo_2 = (real_vn_fee - generic_fee) + input_fee + MicroMinotari(100);
        insert_spendable_utxo(&conn, &account, utxo_1, 1);
        insert_spendable_utxo(&conn, &account, utxo_2, 2);

        let prepared = build_vn_pay_to_self_tx(
            &account,
            &mut conn,
            network,
            "pass",
            features,
            fee_per_gram,
            None,
            Some("vn-size-key".to_string()),
            3600,
            10,
            IdempotencyOperation::ValidatorNodeRegistration,
        )
        .expect("measuring VN output features selects both UTXOs and succeeds");

        assert_eq!(prepared.info.inputs.len(), 2);
    }
}
