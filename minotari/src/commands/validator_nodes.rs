//! CLI handlers for validator node commands.
//!
//! Each handler parses its CLI inputs, builds the appropriate params struct,
//! calls the transaction constructor, then signs, persists, and broadcasts the result.

use std::{fs, path::PathBuf};

use crate::{
    db::{self, AccountRow, init_db},
    http::WalletHttpClient,
    models::PendingTransactionStatus,
    transactions::validator_node::{
        eviction::{ValidatorNodeEvictionParams, create_validator_node_eviction_tx},
        exit::{ValidatorNodeExitParams, create_validator_node_exit_tx},
        registration::{ValidatorNodeRegistrationParams, create_validator_node_registration_tx},
    },
};
use anyhow::anyhow;
use log::info;
use rusqlite::Connection;
use tari_common::configuration::Network;
use tari_common_types::{
    epoch::VnEpoch,
    types::{CompressedPublicKey, CompressedSignature, PrivateKey},
};
use tari_transaction_components::{
    consensus::ConsensusConstantsBuilder,
    offline_signing::{models::PrepareOneSidedTransactionForSigningResult, sign_locked_transaction},
    tari_amount::MicroMinotari,
};
use tari_utilities::byte_array::ByteArray;

// ── Parsing helpers ──────────────────────────────────────────────────────────

fn parse_compressed_public_key(hex_str: &str, field_name: &str) -> Result<CompressedPublicKey, anyhow::Error> {
    let bytes = hex::decode(hex_str).map_err(|e| anyhow!("Invalid {} hex: {}", field_name, e))?;
    CompressedPublicKey::from_canonical_bytes(&bytes).map_err(|e| anyhow!("Invalid {}: {}", field_name, e))
}

/// Parses the three hex strings that describe a VN Schnorr signature.
///
/// Returns `(vn_public_key, signature)` where the signature bundles the public nonce
/// and the scalar component.
fn parse_vn_signature(
    pk_hex: &str,
    nonce_hex: &str,
    sig_hex: &str,
) -> Result<(CompressedPublicKey, CompressedSignature), anyhow::Error> {
    let vn_public_key = parse_compressed_public_key(pk_hex, "vn-public-key")?;
    let nonce = parse_compressed_public_key(nonce_hex, "vn-sig-nonce")?;
    let sig_bytes = hex::decode(sig_hex).map_err(|e| anyhow!("Invalid vn-sig hex: {}", e))?;
    let sig_scalar =
        PrivateKey::from_canonical_bytes(&sig_bytes).map_err(|e| anyhow!("Invalid vn-sig scalar: {}", e))?;
    Ok((vn_public_key, CompressedSignature::new(nonce, sig_scalar)))
}

fn parse_sidechain_deployment_key(key: Option<String>) -> Result<Option<PrivateKey>, anyhow::Error> {
    key.map(|k| {
        let bytes = hex::decode(&k).map_err(|e| anyhow!("Invalid sidechain-deployment-key hex: {}", e))?;
        PrivateKey::from_canonical_bytes(&bytes).map_err(|e| anyhow!("Invalid sidechain-deployment-key: {}", e))
    })
    .transpose()
}

// ── Sign + persist + broadcast ────────────────────────────────────────────────

/// Takes this command's fund reservation, so it cannot be broadcast twice or
/// over UTXOs the wallet has already released.
///
/// The lookup and the status write used to be two autocommit statements. Against
/// a database a daemon is also serving, its unlocker could expire the
/// reservation in the gap and release the outputs; the unguarded write then
/// flipped the row back to `Completed` and the broadcast spent UTXOs the wallet
/// now recorded as `Unspent`, leaving the next send free to build a conflicting
/// spend. The conditional update closes that gap.
///
/// A missing row is the same failure caught one statement earlier: the caller
/// locked funds under this key moments ago, so the reservation can only be
/// absent because it was expired or cancelled. This used to skip the whole block
/// and broadcast anyway, putting a transaction on the network with nothing
/// recording it; now it aborts before anything is sent.
fn claim_reservation_for_broadcast(
    conn: &Connection,
    account_id: i64,
    idempotency_key: &str,
    tx_kind: &str,
) -> Result<String, anyhow::Error> {
    let pending_tx =
        db::find_pending_transaction_by_idempotency_key(conn, idempotency_key, account_id)?.ok_or_else(|| {
            anyhow!(
                "The {} reservation for idempotency key '{}' is no longer active; nothing was broadcast",
                tx_kind,
                idempotency_key
            )
        })?;

    let pending_tx_id = pending_tx.id.to_string();
    if !db::update_pending_transaction_status_if(
        conn,
        &pending_tx_id,
        &PendingTransactionStatus::Pending,
        PendingTransactionStatus::Completed,
    )? {
        return Err(anyhow!(
            "The {} reservation for idempotency key '{}' was released before it could be broadcast; nothing was sent",
            tx_kind,
            idempotency_key
        ));
    }

    Ok(pending_tx_id)
}

/// Signs an unsigned VN transaction, saves it to the DB, and broadcasts it.
///
/// This is the common post-processing step shared by all three VN commands.
/// `tx_kind` is used in log/error messages (e.g. `"VN registration"`).
#[allow(clippy::too_many_arguments)]
async fn sign_save_and_broadcast(
    unsigned_result: PrepareOneSidedTransactionForSigningResult,
    account: &AccountRow,
    conn: &Connection,
    password: &str,
    network: Network,
    idempotency_key: &str,
    base_url: &str,
    tx_kind: &str,
) -> Result<(), anyhow::Error> {
    let key_manager = account.get_key_manager(password)?;
    let consensus_constants = ConsensusConstantsBuilder::new(network).build();
    let signed_result = sign_locked_transaction(&key_manager, consensus_constants, network, unsigned_result)
        .map_err(|e| anyhow!("Failed to sign {} transaction: {}", tx_kind, e))?;

    let completed_tx_id = signed_result.signed_transaction.tx_id;
    let kernel_excess = signed_result
        .signed_transaction
        .transaction
        .body()
        .kernels()
        .first()
        .map(|k| k.excess.as_bytes().to_vec())
        .unwrap_or_default();
    let serialized_tx = serde_json::to_vec(&signed_result.signed_transaction.transaction)
        .map_err(|e| anyhow!("Failed to serialize transaction: {}", e))?;
    let sent_output_hash = signed_result.signed_transaction.sent_hashes.first().map(hex::encode);

    let pending_tx_id = claim_reservation_for_broadcast(conn, account.id, idempotency_key, tx_kind)?;
    db::create_completed_transaction(
        conn,
        account.id,
        &pending_tx_id,
        &kernel_excess,
        &serialized_tx,
        sent_output_hash,
        completed_tx_id,
    )?;

    let client = WalletHttpClient::new(base_url.parse()?)?;
    let response = client
        .submit_transaction(signed_result.signed_transaction.transaction)
        .await;

    match response {
        Ok(r) if r.accepted => {
            db::mark_completed_transaction_as_broadcasted(conn, completed_tx_id, 1)?;
            info!(target: "audit", tx_id = completed_tx_id.to_string().as_str(); "{} broadcasted", tx_kind);
            println!("{} transaction broadcasted. tx_id={}", tx_kind, completed_tx_id);
        },
        Ok(r) => return Err(anyhow!("Transaction rejected by network: {}", r.rejection_reason)),
        Err(e) => return Err(anyhow!("Broadcast failed: {}", e)),
    }

    Ok(())
}

// ── Shared command runner ─────────────────────────────────────────────────────

/// Handles the shared boilerplate for all three VN commands: initialise the
/// database, fetch the account, call the operation-specific transaction
/// constructor, then sign, persist, and broadcast the result.
///
/// `create_tx` is a closure that captures the operation-specific params and
/// returns an unsigned transaction ready for signing.
#[allow(clippy::too_many_arguments)]
async fn run_vn_command<F>(
    database_file: PathBuf,
    account_name: &str,
    network: Network,
    password: &str,
    idempotency_key: Option<String>,
    seconds_to_lock: u64,
    confirmation_window: u64,
    base_url: &str,
    tx_kind: &str,
    create_tx: F,
) -> Result<(), anyhow::Error>
where
    F: FnOnce(
        &AccountRow,
        &mut Connection,
        Network,
        &str,
        Option<String>,
        u64,
        u64,
    ) -> Result<PrepareOneSidedTransactionForSigningResult, anyhow::Error>,
{
    let idempotency_key = idempotency_key.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let pool = init_db(database_file)?;
    // One connection for the command: the VN transaction is built on it and the result is
    // then saved and broadcast with it, so nothing is held idle while `lock` waits.
    let mut conn = pool.get()?;
    let account =
        db::get_account_by_name(&conn, account_name)?.ok_or_else(|| anyhow!("Account not found: {}", account_name))?;

    let unsigned_result = create_tx(
        &account,
        &mut conn,
        network,
        password,
        Some(idempotency_key.clone()),
        seconds_to_lock,
        confirmation_window,
    )?;

    sign_save_and_broadcast(
        unsigned_result,
        &account,
        &conn,
        password,
        network,
        &idempotency_key,
        base_url,
        tx_kind,
    )
    .await
}

// ── Public handlers ───────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
pub async fn handle_register_validator_node(
    vn_public_key: String,
    vn_sig_nonce: String,
    vn_sig: String,
    claim_public_key: String,
    max_epoch: u64,
    fee_per_gram: u64,
    payment_id: Option<String>,
    sidechain_deployment_key: Option<String>,
    database_file: PathBuf,
    account_name: String,
    network: Network,
    password: String,
    idempotency_key: Option<String>,
    seconds_to_lock: u64,
    confirmation_window: u64,
    base_url: String,
) -> Result<(), anyhow::Error> {
    let (vn_public_key, vn_signature) = parse_vn_signature(&vn_public_key, &vn_sig_nonce, &vn_sig)?;
    let claim_public_key = parse_compressed_public_key(&claim_public_key, "claim-public-key")?;
    let sidechain_deployment_key = parse_sidechain_deployment_key(sidechain_deployment_key)?;

    let params = ValidatorNodeRegistrationParams {
        validator_node_public_key: vn_public_key,
        validator_node_signature: vn_signature,
        claim_public_key,
        max_epoch: VnEpoch(max_epoch),
        fee_per_gram: MicroMinotari(fee_per_gram),
        payment_id,
        sidechain_deployment_key,
    };

    run_vn_command(
        database_file,
        &account_name,
        network,
        &password,
        idempotency_key,
        seconds_to_lock,
        confirmation_window,
        &base_url,
        "VN registration",
        |account, conn, network, password, idempotency_key, seconds_to_lock, confirmation_window| {
            create_validator_node_registration_tx(
                account,
                params,
                conn,
                network,
                password,
                idempotency_key,
                seconds_to_lock,
                confirmation_window,
            )
        },
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn handle_submit_validator_node_exit(
    vn_public_key: String,
    vn_sig_nonce: String,
    vn_sig: String,
    max_epoch: u64,
    fee_per_gram: u64,
    payment_id: Option<String>,
    sidechain_deployment_key: Option<String>,
    database_file: PathBuf,
    account_name: String,
    network: Network,
    password: String,
    idempotency_key: Option<String>,
    seconds_to_lock: u64,
    confirmation_window: u64,
    base_url: String,
) -> Result<(), anyhow::Error> {
    let (vn_public_key, vn_signature) = parse_vn_signature(&vn_public_key, &vn_sig_nonce, &vn_sig)?;
    let sidechain_deployment_key = parse_sidechain_deployment_key(sidechain_deployment_key)?;

    let params = ValidatorNodeExitParams {
        validator_node_public_key: vn_public_key,
        validator_node_signature: vn_signature,
        max_epoch: VnEpoch(max_epoch),
        fee_per_gram: MicroMinotari(fee_per_gram),
        payment_id,
        sidechain_deployment_key,
    };

    run_vn_command(
        database_file,
        &account_name,
        network,
        &password,
        idempotency_key,
        seconds_to_lock,
        confirmation_window,
        &base_url,
        "VN exit",
        |account, conn, network, password, idempotency_key, seconds_to_lock, confirmation_window| {
            create_validator_node_exit_tx(
                account,
                params,
                conn,
                network,
                password,
                idempotency_key,
                seconds_to_lock,
                confirmation_window,
            )
        },
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn handle_submit_validator_eviction_proof(
    proof_file: PathBuf,
    fee_per_gram: u64,
    payment_id: Option<String>,
    sidechain_deployment_key: Option<String>,
    database_file: PathBuf,
    account_name: String,
    network: Network,
    password: String,
    idempotency_key: Option<String>,
    seconds_to_lock: u64,
    confirmation_window: u64,
    base_url: String,
) -> Result<(), anyhow::Error> {
    let proof_json = fs::read_to_string(&proof_file)
        .map_err(|e| anyhow!("Failed to read proof file '{}': {}", proof_file.display(), e))?;
    let eviction_proof: tari_sidechain::EvictionProof =
        serde_json::from_str(&proof_json).map_err(|e| anyhow!("Failed to parse eviction proof JSON: {}", e))?;

    let sidechain_deployment_key = parse_sidechain_deployment_key(sidechain_deployment_key)?;

    let params = ValidatorNodeEvictionParams {
        eviction_proof,
        fee_per_gram: MicroMinotari(fee_per_gram),
        payment_id,
        sidechain_deployment_key,
    };

    run_vn_command(
        database_file,
        &account_name,
        network,
        &password,
        idempotency_key,
        seconds_to_lock,
        confirmation_window,
        &base_url,
        "VN eviction proof",
        |account, conn, network, password, idempotency_key, seconds_to_lock, confirmation_window| {
            create_validator_node_eviction_tx(
                account,
                params,
                conn,
                network,
                password,
                idempotency_key,
                seconds_to_lock,
                confirmation_window,
            )
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        db::{SqlitePool, create_account, get_account_by_name},
        transactions::idempotency::{IdempotencyBinding, IdempotencyOperation, RequestFingerprint},
    };
    use chrono::{TimeDelta, Utc};
    use rusqlite::named_params;
    use tari_common_types::seeds::cipher_seed::CipherSeed;
    use tari_transaction_components::key_manager::wallet_types::{SeedWordsWallet, WalletType};
    use tempfile::tempdir;

    const KEY: &str = "vn-key";

    /// A migrated database holding one account and one live VN reservation.
    fn setup_reservation() -> (SqlitePool, i64, tempfile::TempDir) {
        let temp = tempdir().expect("temp dir");
        let pool = init_db(temp.path().join("test.db")).expect("init db");
        let conn = pool.get().expect("get conn");

        let wallet =
            WalletType::SeedWords(SeedWordsWallet::construct_new(CipherSeed::random()).expect("construct wallet"));
        create_account(&conn, "test", &wallet, "pass").expect("create account");
        let account_id = get_account_by_name(&conn, "test")
            .expect("get account")
            .expect("account exists")
            .id;

        let operation = IdempotencyOperation::ValidatorNodeRegistration;
        let binding = IdempotencyBinding::new(Some(KEY.to_string()), operation, RequestFingerprint::new(operation));
        db::create_pending_transaction(
            &conn,
            KEY,
            &binding,
            account_id,
            false,
            MicroMinotari(1_000),
            MicroMinotari(0),
            MicroMinotari(0),
            Utc::now() + TimeDelta::hours(1),
        )
        .expect("create reservation");

        drop(conn);
        (pool, account_id, temp)
    }

    fn status_of(pool: &SqlitePool) -> String {
        pool.get()
            .expect("conn")
            .query_row(
                "SELECT status FROM pending_transactions WHERE idempotency_key = :key",
                named_params! { ":key": KEY },
                |row| row.get(0),
            )
            .expect("reservation exists")
    }

    #[test]
    fn a_live_reservation_is_claimed_once() {
        let (pool, account_id, _temp) = setup_reservation();

        claim_reservation_for_broadcast(&pool.get().expect("conn"), account_id, KEY, "VN registration")
            .expect("the reservation is live");
        assert_eq!(status_of(&pool), PendingTransactionStatus::Completed.to_string());

        claim_reservation_for_broadcast(&pool.get().expect("conn"), account_id, KEY, "VN registration")
            .expect_err("a reservation cannot be claimed twice");
    }

    /// A reservation the unlocker released must abort the broadcast.
    ///
    /// The lookup and the status write were two autocommit statements, so the
    /// daemon's unlocker could expire the reservation and release its outputs in
    /// between. The unguarded write then flipped the row back to `Completed` and
    /// the transaction went to the network over UTXOs the wallet had already
    /// handed back to the next send.
    #[test]
    fn a_reservation_released_before_the_claim_aborts_the_broadcast() {
        let (pool, account_id, _temp) = setup_reservation();

        // The unlocker gets there first.
        let conn = pool.get().expect("conn");
        db::update_pending_transaction_status(&conn, &pending_id(&pool), PendingTransactionStatus::Expired)
            .expect("expire the reservation");
        drop(conn);

        claim_reservation_for_broadcast(&pool.get().expect("conn"), account_id, KEY, "VN registration")
            .expect_err("a released reservation must not be broadcast");
        assert_eq!(
            status_of(&pool),
            PendingTransactionStatus::Expired.to_string(),
            "the claim must not resurrect a released reservation",
        );
    }

    fn pending_id(pool: &SqlitePool) -> String {
        pool.get()
            .expect("conn")
            .query_row(
                "SELECT id FROM pending_transactions WHERE idempotency_key = :key",
                named_params! { ":key": KEY },
                |row| row.get(0),
            )
            .expect("reservation exists")
    }
}
