// Copyright 2026 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashSet, str::FromStr};

use anyhow::{Context, anyhow};
use borsh::BorshDeserialize;
use tari_common_types::{
    transaction::TxId,
    types::{ComAndPubSignature, CompressedCommitment, CompressedPublicKey, FixedHash, PrivateKey, RangeProof},
};
use tari_script::{ExecutionStack, TariScript};
use tari_transaction_components::{
    MicroMinotari,
    key_manager::TariKeyId,
    transaction_components::{
        EncryptedData, OutputFeatures, TransactionOutputVersion, WalletOutput, covenants::Covenant,
        memo_field::MemoField,
    },
};
use tari_utilities::ByteArray;

use super::console_db::ConsoleOutput;

#[derive(Debug, Clone)]
pub struct LegacyKeyBlocker {
    pub output_hash_hex: String,
    pub field_name: &'static str,
}

#[derive(Debug, Clone)]
pub struct ConvertedOutput {
    pub wallet_output: WalletOutput,
    pub output_hash: FixedHash,
    pub mined_block_hash: FixedHash,
    pub mined_height: u64,
    pub mined_timestamp: chrono::NaiveDateTime,
    pub original_received_in_tx_id: Option<i64>,
    pub destination_tx_id: TxId,
}

pub fn detect_legacy_key_blocker(output: &ConsoleOutput) -> Option<LegacyKeyBlocker> {
    let output_hash_hex = hex::encode(&output.hash);

    if TariKeyId::from_str(&output.spending_key).is_err() {
        return Some(LegacyKeyBlocker {
            output_hash_hex,
            field_name: "spending_key",
        });
    }

    if TariKeyId::from_str(&output.script_private_key).is_err() {
        return Some(LegacyKeyBlocker {
            output_hash_hex,
            field_name: "script_private_key",
        });
    }

    None
}

pub fn convert_output(output: &ConsoleOutput) -> Result<ConvertedOutput, anyhow::Error> {
    let commitment_mask_key_id = TariKeyId::from_str(&output.spending_key).map_err(|_| {
        anyhow!(
            "Legacy key format detected for output {} in spending_key",
            hex::encode(&output.hash)
        )
    })?;
    let script_key_id = TariKeyId::from_str(&output.script_private_key).map_err(|_| {
        anyhow!(
            "Legacy key format detected for output {} in script_private_key",
            hex::encode(&output.hash)
        )
    })?;

    let output_hash = FixedHash::try_from(output.hash.clone())
        .map_err(|_| anyhow!("Invalid output hash bytes for {}", hex::encode(&output.hash)))?;
    let mined_block_hash = FixedHash::try_from(
        output
            .mined_in_block
            .clone()
            .ok_or_else(|| anyhow!("Source output {} is missing mined_in_block", hex::encode(&output.hash)))?,
    )
    .map_err(|_| anyhow!("Invalid mined_in_block hash for {}", hex::encode(&output.hash)))?;
    let mined_height = output
        .mined_height
        .ok_or_else(|| anyhow!("Source output {} is missing mined_height", hex::encode(&output.hash)))?
        .try_into()
        .map_err(|_| anyhow!("Invalid mined_height for {}", hex::encode(&output.hash)))?;
    let mined_timestamp = output
        .mined_timestamp
        .ok_or_else(|| anyhow!("Source output {} is missing mined_timestamp", hex::encode(&output.hash)))?;

    let features: OutputFeatures = serde_json::from_str(&output.features_json)
        .with_context(|| format!("Failed to parse features_json for output {}", hex::encode(&output.hash)))?;
    let covenant_slice = output.covenant.as_slice();
    let covenant = Covenant::deserialize(&mut &*covenant_slice)
        .map_err(|e| anyhow!("Invalid covenant for output {}: {}", hex::encode(&output.hash), e))?;
    let encrypted_data = EncryptedData::from_bytes(&output.encrypted_data).with_context(|| {
        format!(
            "Failed to parse encrypted_data for output {}",
            hex::encode(&output.hash)
        )
    })?;
    let payment_id = match &output.payment_id {
        Some(bytes) => MemoField::from_bytes(bytes),
        None => MemoField::new_empty(),
    };
    let commitment = CompressedCommitment::from_vec(&output.commitment).map_err(|e| {
        anyhow!(
            "Invalid commitment bytes for output {}: {:?}",
            hex::encode(&output.hash),
            e
        )
    })?;
    let sender_offset_public_key = CompressedPublicKey::from_vec(&output.sender_offset_public_key).map_err(|e| {
        anyhow!(
            "Invalid sender_offset_public_key bytes for output {}: {:?}",
            hex::encode(&output.hash),
            e
        )
    })?;
    let metadata_signature = ComAndPubSignature::new(
        CompressedCommitment::from_vec(&output.metadata_signature_ephemeral_commitment).map_err(|e| {
            anyhow!(
                "Invalid metadata_signature_ephemeral_commitment for output {}: {:?}",
                hex::encode(&output.hash),
                e
            )
        })?,
        CompressedPublicKey::from_vec(&output.metadata_signature_ephemeral_pubkey).map_err(|e| {
            anyhow!(
                "Invalid metadata_signature_ephemeral_pubkey for output {}: {:?}",
                hex::encode(&output.hash),
                e
            )
        })?,
        PrivateKey::from_vec(&output.metadata_signature_u_a).map_err(|e| {
            anyhow!(
                "Invalid metadata_signature_u_a for output {}: {:?}",
                hex::encode(&output.hash),
                e
            )
        })?,
        PrivateKey::from_vec(&output.metadata_signature_u_x).map_err(|e| {
            anyhow!(
                "Invalid metadata_signature_u_x for output {}: {:?}",
                hex::encode(&output.hash),
                e
            )
        })?,
        PrivateKey::from_vec(&output.metadata_signature_u_y).map_err(|e| {
            anyhow!(
                "Invalid metadata_signature_u_y for output {}: {:?}",
                hex::encode(&output.hash),
                e
            )
        })?,
    );
    let range_proof = output
        .rangeproof
        .as_ref()
        .map(|bytes| RangeProof::from_canonical_bytes(bytes))
        .transpose()
        .map_err(|e| anyhow!("Invalid rangeproof for output {}: {:?}", hex::encode(&output.hash), e))?;

    let wallet_output = WalletOutput::new_from_parts(
        TransactionOutputVersion::get_current_version(),
        MicroMinotari::from(output.value as u64),
        commitment_mask_key_id,
        features,
        TariScript::from_bytes(&output.script)
            .with_context(|| format!("Invalid script for output {}", hex::encode(&output.hash)))?,
        ExecutionStack::from_bytes(&output.input_data)
            .with_context(|| format!("Invalid input_data for output {}", hex::encode(&output.hash)))?,
        script_key_id,
        sender_offset_public_key,
        metadata_signature,
        output.script_lock_height as u64,
        covenant,
        encrypted_data,
        MicroMinotari::from(output.minimum_value_promise as u64),
        range_proof,
        payment_id,
        output_hash,
        commitment,
    );

    Ok(ConvertedOutput {
        wallet_output,
        output_hash,
        mined_block_hash,
        mined_height,
        mined_timestamp,
        original_received_in_tx_id: output.original_received_in_tx_id(),
        destination_tx_id: TxId::new_deterministic(&[], &[0u8; 32]),
    })
}

pub fn assign_destination_tx_ids(outputs: &mut [ConvertedOutput], account_view_key: &PrivateKey) {
    let mut active_received_tx_ids = HashSet::new();

    for output in outputs {
        output.destination_tx_id = match output.original_received_in_tx_id {
            Some(source_tx_id) if active_received_tx_ids.insert(source_tx_id) => TxId::from(source_tx_id as u64),
            _ => {
                // Active outputs have a unique index on outputs.tx_id, so duplicate received_in_tx_id values need a
                // deterministic fallback that matches the wallet's normal scanner behavior.
                TxId::new_deterministic(account_view_key.as_bytes(), &output.wallet_output.output_hash())
            },
        };
    }
}

trait ConsoleOutputExt {
    fn original_received_in_tx_id(&self) -> Option<i64>;
}

impl ConsoleOutputExt for ConsoleOutput {
    fn original_received_in_tx_id(&self) -> Option<i64> {
        self.received_in_tx_id
    }
}
