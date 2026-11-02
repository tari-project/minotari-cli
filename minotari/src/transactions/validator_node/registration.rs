//! Validator node registration transaction construction.
//!
//! This module provides functionality for creating validator node registration
//! transactions on the Tari base layer. The registration embeds the validator
//! node's public key and signature into a pay-to-self transaction with special
//! [`OutputFeatures`], locking a minimum deposit defined by consensus constants.
//!
//! # Pattern
//!
//! The caller pre-computes the [`ValidatorNodeSignature`] on the validator node
//! side, then passes the public key and signature to the wallet for transaction
//! construction. Only [`WalletType::SeedWords`] wallets are supported.
//!
//! # Flow
//!
//! 1. Validate the pre-computed [`ValidatorNodeSignature`]
//! 2. Build [`OutputFeatures::for_validator_node_registration`]
//! 3. Lock the deposit UTXOs via [`FundLocker`] and prepare the transaction for signing

use anyhow::anyhow;
use rusqlite::Connection;
use tari_common::configuration::Network;
use tari_common_types::{
    epoch::VnEpoch,
    types::{CompressedPublicKey, CompressedSignature, PrivateKey},
};
use tari_transaction_components::{
    key_manager::wallet_types::WalletType,
    offline_signing::models::PrepareOneSidedTransactionForSigningResult,
    tari_amount::MicroMinotari,
    transaction_components::{OutputFeatures, ValidatorNodeSignature},
};

use super::common::build_vn_pay_to_self_tx;
use crate::db::AccountRow;
use crate::transactions::idempotency::IdempotencyOperation;

/// Parameters for validator node registration
///
/// The `validator_node_signature` must be pre-computed by the validator node using
/// [`ValidatorNodeSignature::sign_for_registration`] with the node's private key.
pub struct ValidatorNodeRegistrationParams {
    /// The validator node's public key (the signing key used to create `validator_node_signature`).
    pub validator_node_public_key: CompressedPublicKey,
    /// Pre-computed Schnorr signature over `(network | vn_pk | nonce | sidechain_pk? | claim_pk | max_epoch)`.
    pub validator_node_signature: CompressedSignature,
    /// Public key used to claim validator node rewards.
    pub claim_public_key: CompressedPublicKey,
    /// Maximum epoch for replay protection.
    pub max_epoch: VnEpoch,
    /// Fee rate in MicroMinotari per gram.
    pub fee_per_gram: MicroMinotari,
    /// Optional payment memo attached to the transaction.
    pub payment_id: Option<String>,
    /// Optional sidechain deployment private key. If provided, derives the sidechain public key
    /// included in the signature message and creates a [`SideChainId`] proof.
    pub sidechain_deployment_key: Option<PrivateKey>,
}

/// Creates an unsigned validator node registration transaction.
///
/// The returned [`PrepareOneSidedTransactionForSigningResult`] can be written to a
/// file for external signing, following the same flow as [`CreateUnsignedTransaction`].
///
/// # Errors
///
/// Returns an error if:
/// - The account is not a [`WalletType::SeedWords`] wallet
/// - The validator node signature is invalid
/// - There are insufficient funds for the consensus-required deposit
/// - Transaction construction fails
#[allow(clippy::too_many_arguments)]
pub fn create_validator_node_registration_tx(
    account: &AccountRow,
    params: ValidatorNodeRegistrationParams,
    conn: &mut Connection,
    network: Network,
    password: &str,
    idempotency_key: Option<String>,
    seconds_to_lock: u64,
    confirmation_window: u64,
) -> Result<PrepareOneSidedTransactionForSigningResult, anyhow::Error> {
    let wallet_type = account.decrypt_wallet_type(password)?;
    if !matches!(wallet_type, WalletType::SeedWords(_)) {
        return Err(anyhow!(
            "Validator node registration requires a SeedWords wallet, not a view-only wallet"
        ));
    }

    if crate::utils::crypto::is_identity_public_key(&params.claim_public_key) {
        return Err(anyhow!(
            "claim_public_key is the identity element; validator node rewards sent to it could never be claimed"
        ));
    }

    let vn_sig = ValidatorNodeSignature::new(params.validator_node_public_key, params.validator_node_signature);
    let sidechain_pk = params
        .sidechain_deployment_key
        .as_ref()
        .map(CompressedPublicKey::from_secret_key);
    if !vn_sig.is_valid_registration_signature_for(
        network.as_byte(),
        sidechain_pk.as_ref(),
        &params.claim_public_key,
        params.max_epoch,
    ) {
        return Err(anyhow!("Invalid validator node registration signature"));
    }

    let output_features = OutputFeatures::for_validator_node_registration(
        vn_sig,
        params.claim_public_key,
        params.sidechain_deployment_key.as_ref(),
        params.max_epoch,
    );

    build_vn_pay_to_self_tx(
        account,
        conn,
        network,
        password,
        output_features,
        params.fee_per_gram,
        params.payment_id.as_deref(),
        idempotency_key,
        seconds_to_lock,
        confirmation_window,
        IdempotencyOperation::ValidatorNodeRegistration,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{create_account, get_account_by_name, init_db};
    use tari_common_types::seeds::cipher_seed::CipherSeed;
    use tari_transaction_components::key_manager::wallet_types::SeedWordsWallet;
    use tempfile::tempdir;

    #[test]
    fn registration_rejects_identity_claim_public_key() {
        let temp = tempdir().unwrap();
        let pool = init_db(temp.path().join("vn_reg.db")).unwrap();
        let mut conn = pool.get().unwrap();

        let wallet = WalletType::SeedWords(SeedWordsWallet::construct_new(CipherSeed::random()).unwrap());
        create_account(&conn, "vn", &wallet, "pass").unwrap();
        let account = get_account_by_name(&conn, "vn").unwrap().unwrap();

        let vn_sk = PrivateKey::from(7u64);
        let identity_claim_pk = CompressedPublicKey::default();
        let max_epoch = VnEpoch(50);
        let vn_sig = ValidatorNodeSignature::sign_for_registration(
            &vn_sk,
            Network::LocalNet.as_byte(),
            None,
            &identity_claim_pk,
            max_epoch,
        );

        let params = ValidatorNodeRegistrationParams {
            validator_node_public_key: vn_sig.public_key().clone(),
            validator_node_signature: vn_sig.signature().clone(),
            claim_public_key: identity_claim_pk,
            max_epoch,
            fee_per_gram: MicroMinotari(5),
            payment_id: None,
            sidechain_deployment_key: None,
        };

        let err = create_validator_node_registration_tx(
            &account,
            params,
            &mut conn,
            Network::LocalNet,
            "pass",
            Some("vn-reg-id-key".to_string()),
            3600,
            10,
        )
        .expect_err("identity claim_public_key must be rejected");
        assert!(err.to_string().contains("identity element"), "unexpected error: {err}");
    }
}
