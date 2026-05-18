// Copyright 2026 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

//! Convert a legacy `ConsoleOutputRow` into a new-format `WalletOutput`.
//!
//! The legacy console wallet stores outputs as raw column bytes in a Diesel-managed
//! SQLite table. The new minotari-cli stores them as `WalletOutput` structs
//! serialized to JSON. This module bridges the gap.

use anyhow::{Context, anyhow};
use tari_common_types::seeds::cipher_seed::CipherSeed;
use tari_crypto::keys::SecretKey;
use tari_transaction_components::transaction_components::{OutputType, WalletOutput, OutputFeatures, CommitmentFactory};
use tari_transaction_components::types::{Commitment, PrivateKey, PublicKey};
use tari_utilities::ByteArray;

use super::console_db::ConsoleOutputRow;

/// Convert a legacy console wallet output row into a new `WalletOutput`.
///
/// # Errors
///
/// Returns an error if any cryptographic material cannot be deserialized
/// (corrupted DB, wrong schema version, etc.).
pub fn convert_output(legacy: &ConsoleOutputRow, _cipher_seed: &CipherSeed) -> anyhow::Result<WalletOutput> {
    // Parse the spending key (hex-encoded private key)
    let spending_key = PrivateKey::from_hex(&legacy.spending_key)
        .with_context(|| format!("Invalid spending_key hex in output (value={})", legacy.value))?;

    // Parse the sender offset public key
    let sender_offset_public_key = PublicKey::from_bytes(&legacy.sender_offset_public_key)
        .with_context(|| "Invalid sender_offset_public_key bytes")?;

    // Parse the commitment
    let commitment = Commitment::from_bytes(&legacy.commitment)
        .with_context(|| "Invalid commitment bytes")?;

    // Parse output type
    let output_type = match legacy.output_type {
        0 => OutputType::Standard,
        1 => OutputType::Coinbase,
        2 => OutputType::Burn,
        3 => OutputType::ValidatorNodeRegistration,
        4 => OutputType::CodeTemplateRegistration,
        _ => OutputType::Standard,
    };

    // Parse features from JSON
    let features: OutputFeatures = if legacy.features_json.is_empty() {
        OutputFeatures::default()
    } else {
        serde_json::from_str(&legacy.features_json).unwrap_or_default()
    };

    // Parse script private key
    let script_private_key = if legacy.script_private_key.is_empty() {
        PrivateKey::default()
    } else {
        PrivateKey::from_hex(&legacy.script_private_key).unwrap_or_default()
    };

    // Build the metadata signature components
    // The legacy DB stores the signature as separate u_a, u_x, u_y + ephemeral components
    // We reconstruct the aggregated signature
    let metadata_signature = tari_transaction_components::transaction_components::MetadataSignature {
        public_nonce_commitment: Commitment::from_bytes(
            &legacy.metadata_signature_ephemeral_commitment
        ).unwrap_or_else(|_| Commitment::from_bytes(&[0u8; 32]).unwrap()),
        public_nonce_pubkey: PublicKey::from_bytes(
            &legacy.metadata_signature_ephemeral_pubkey
        ).unwrap_or_else(|_| PublicKey::default()),
        signature_u: PrivateKey::from_bytes(&legacy.metadata_signature_u_x)
            .unwrap_or_else(|_| PrivateKey::default()),
        signature_v: PrivateKey::from_bytes(&legacy.metadata_signature_u_y)
            .unwrap_or_else(|_| PrivateKey::default()),
    };

    // Build the WalletOutput
    let wallet_output = WalletOutput::new(
        output_type,                                    // output_type
        legacy.value as u64,                            // value
        spending_key,                                   // spending_key
        commitment,                                     // commitment
        features,                                       // features
        legacy.script.clone(),                          // script
        legacy.input_data.clone(),                      // input_data
        script_private_key,                             // script_private_key
        legacy.script_lock_height as u64,               // script_lock_height
        sender_offset_public_key,                       // sender_offset_public_key
        metadata_signature,                             // metadata_signature
        Default::default(),                             // covenant (simplified)
        legacy.covenant.clone().try_into().unwrap_or_default(), // encrypted_data (use covenant bytes as placeholder)
        legacy.mined_height.map(|h| h as u64),          // mined_height
        legacy.rangeproof.clone(),                      // rangeproof
    );

    Ok(wallet_output)
}
