//! Convert a console wallet `outputs` row into the type the new wallet stores
//! (`WalletOutput` JSON blob plus surrounding metadata).
//!
//! The console wallet decomposes a `WalletOutput` across about 20 columns;
//! the new wallet keeps the whole thing as a single serde_json string with
//! a few indexed fields alongside (commitment hash, value, status, etc.).
//!
//! Mirrors the logic of
//! `tari_wallet::output_manager_service::storage::sqlite_db::output_sql::OutputSql::to_db_wallet_output`,
//! but driven directly off the raw column bytes rather than the Diesel
//! `OutputSql` struct, so that minotari-cli does not need to depend on the
//! `tari_wallet` crate.

use std::str::FromStr;

use anyhow::anyhow;
use tari_common_types::types::{
    ComAndPubSignature, CompressedCommitment, CompressedPublicKey, FixedHash, PrivateKey, RangeProof,
};
use tari_script::{ExecutionStack, TariScript};
use tari_transaction_components::{
    MicroMinotari,
    key_manager::TariKeyId,
    transaction_components::{
        EncryptedData, OutputFeatures, TransactionOutputVersion, WalletOutput,
        covenants::Covenant, memo_field::MemoField,
    },
};
use tari_utilities::ByteArray;

use super::console_db::ConsoleOutputRow;

/// What the migration produces for one output: the reconstructed `WalletOutput`
/// itself plus the metadata the new wallet's `outputs` table needs alongside.
pub struct ConvertedOutput {
    pub wallet_output: WalletOutput,
    pub output_hash: FixedHash,
    pub commitment: CompressedCommitment,
    pub value: MicroMinotari,
    pub mined_height: u64,
    pub mined_block_hash: FixedHash,
    pub mined_timestamp: chrono::NaiveDateTime,
    pub received_in_tx_id: Option<u64>,
    pub spent_in_tx_id: Option<u64>,
    pub legacy_status: LegacyOutputStatus,
}

/// The console wallet's `OutputStatus` enum, by integer value. We keep this
/// local because the published `tari_common_types` does not expose it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyOutputStatus {
    Unspent,
    Spent,
    EncumberedToBeReceived,
    EncumberedToBeSpent,
    Invalid,
    CancelledInbound,
    UnspentMinedUnconfirmed,
    ShortTermEncumberedToBeReceived,
    ShortTermEncumberedToBeSpent,
    SpentMinedUnconfirmed,
    NotStored,
}

impl LegacyOutputStatus {
    pub fn from_i32(value: i32) -> Result<Self, anyhow::Error> {
        let status = match value {
            0 => Self::Unspent,
            1 => Self::Spent,
            2 => Self::EncumberedToBeReceived,
            3 => Self::EncumberedToBeSpent,
            4 => Self::Invalid,
            5 => Self::CancelledInbound,
            6 => Self::UnspentMinedUnconfirmed,
            7 => Self::ShortTermEncumberedToBeReceived,
            8 => Self::ShortTermEncumberedToBeSpent,
            9 => Self::SpentMinedUnconfirmed,
            10 => Self::NotStored,
            _ => return Err(anyhow!("Unknown OutputStatus integer value {value}")),
        };
        Ok(status)
    }

    /// Returns true for outputs that should be carried over to the new wallet.
    /// Cancelled / not-stored / invalid outputs are intentionally dropped — they
    /// add no value and would clutter the new wallet's view.
    pub fn is_migratable(self) -> bool {
        matches!(
            self,
            Self::Unspent
                | Self::Spent
                | Self::UnspentMinedUnconfirmed
                | Self::SpentMinedUnconfirmed
                | Self::EncumberedToBeReceived
                | Self::EncumberedToBeSpent
                | Self::ShortTermEncumberedToBeReceived
                | Self::ShortTermEncumberedToBeSpent
        )
    }

    /// True if this output still represents claimable value in the wallet.
    pub fn is_unspent(self) -> bool {
        matches!(
            self,
            Self::Unspent
                | Self::UnspentMinedUnconfirmed
                | Self::EncumberedToBeReceived
                | Self::ShortTermEncumberedToBeReceived
        )
    }

    pub fn is_spent(self) -> bool {
        matches!(self, Self::Spent | Self::SpentMinedUnconfirmed)
    }
}

/// Reconstruct a `WalletOutput` from the source row's raw column bytes.
///
/// Returns `None` if the output should be skipped (e.g. cancelled, not stored,
/// invalid). Returns `Err` if the row's bytes are corrupt — those should fail
/// the migration loudly rather than silently dropping data.
pub fn convert_output(row: &ConsoleOutputRow) -> Result<Option<ConvertedOutput>, anyhow::Error> {
    let legacy_status = LegacyOutputStatus::from_i32(row.status)?;
    if !legacy_status.is_migratable() {
        return Ok(None);
    }

    // Mined-block info is required: an output we never saw on chain has no
    // place in a "ready to use" migrated wallet. The console wallet sets these
    // fields together when an output is mined, so requiring all three is safe.
    let mined_height = row
        .mined_height
        .ok_or_else(|| anyhow!("Output {} has no mined_height; cannot migrate", row_label(row)))?;
    let mined_block_bytes = row
        .mined_in_block
        .as_ref()
        .ok_or_else(|| anyhow!("Output {} has no mined_in_block hash", row_label(row)))?;
    let mined_block_hash = FixedHash::try_from(mined_block_bytes.as_slice())
        .map_err(|e| anyhow!("Output {}: invalid mined_in_block hash: {e}", row_label(row)))?;
    let mined_timestamp = row
        .mined_timestamp
        .ok_or_else(|| anyhow!("Output {} has no mined_timestamp", row_label(row)))?;

    // `WalletOutput` field reconstruction, mirroring
    // `OutputSql::to_db_wallet_output` from the console wallet exactly.
    let features: OutputFeatures = serde_json::from_str(&row.features_json)
        .map_err(|e| anyhow!("Output {}: invalid features_json: {e}", row_label(row)))?;

    let covenant = Covenant::from_bytes(&mut row.covenant.as_slice())
        .map_err(|e| anyhow!("Output {}: bad covenant bytes: {e}", row_label(row)))?;

    let encrypted_data = EncryptedData::from_bytes(&row.encrypted_data)
        .map_err(|e| anyhow!("Output {}: bad encrypted_data: {e}", row_label(row)))?;

    let payment_id = match &row.payment_id {
        Some(bytes) => MemoField::from_bytes(bytes),
        None => MemoField::new_empty(),
    };

    let commitment = CompressedCommitment::from_canonical_bytes(&row.commitment)
        .map_err(|e| anyhow!("Output {}: bad commitment bytes: {e}", row_label(row)))?;

    let output_hash = FixedHash::try_from(row.hash.as_slice())
        .map_err(|e| anyhow!("Output {}: bad hash bytes: {e}", row_label(row)))?;

    // The console wallet supports falling back to a `LegacyTariKeyId` parser if
    // the modern `TariKeyId::from_str` fails. The legacy types live in
    // `tari_transaction_key_manager`, which is not on minotari-cli's dep tree.
    // Rather than pull that in, we surface a clear error and ask the user to
    // run the latest console wallet binary first; that wallet auto-converts
    // legacy key IDs to modern ones on startup.
    let commitment_mask_key_id = TariKeyId::from_str(&row.spending_key).map_err(|e| {
        anyhow!(
            "Output {}: spending_key '{}' is not a recognised TariKeyId ({e}). \
             If this is a very old wallet, open it once with the latest console wallet binary \
             so the on-disk key IDs are upgraded, then retry migration.",
            row_label(row),
            row.spending_key
        )
    })?;
    let script_key_id = TariKeyId::from_str(&row.script_private_key).map_err(|e| {
        anyhow!(
            "Output {}: script_private_key '{}' is not a recognised TariKeyId ({e}).",
            row_label(row),
            row.script_private_key
        )
    })?;

    let metadata_signature = ComAndPubSignature::new(
        CompressedCommitment::from_canonical_bytes(&row.metadata_signature_ephemeral_commitment)
            .map_err(|e| anyhow!("Output {}: bad metadata ephemeral commitment: {e}", row_label(row)))?,
        CompressedPublicKey::from_canonical_bytes(&row.metadata_signature_ephemeral_pubkey)
            .map_err(|e| anyhow!("Output {}: bad metadata ephemeral pubkey: {e}", row_label(row)))?,
        PrivateKey::from_canonical_bytes(&row.metadata_signature_u_a)
            .map_err(|e| anyhow!("Output {}: bad metadata u_a: {e}", row_label(row)))?,
        PrivateKey::from_canonical_bytes(&row.metadata_signature_u_x)
            .map_err(|e| anyhow!("Output {}: bad metadata u_x: {e}", row_label(row)))?,
        PrivateKey::from_canonical_bytes(&row.metadata_signature_u_y)
            .map_err(|e| anyhow!("Output {}: bad metadata u_y: {e}", row_label(row)))?,
    );

    let sender_offset_public_key = CompressedPublicKey::from_canonical_bytes(&row.sender_offset_public_key)
        .map_err(|e| anyhow!("Output {}: bad sender_offset_public_key: {e}", row_label(row)))?;

    let script = TariScript::from_bytes(&row.script)
        .map_err(|e| anyhow!("Output {}: bad script bytes: {e}", row_label(row)))?;
    let input_data = ExecutionStack::from_bytes(&row.input_data)
        .map_err(|e| anyhow!("Output {}: bad input_data bytes: {e}", row_label(row)))?;

    let value = MicroMinotari::from(u64::try_from(row.value).unwrap_or(0));
    let minimum_value_promise = MicroMinotari::from(u64::try_from(row.minimum_value_promise).unwrap_or(0));

    // Range proof is not stored on the console wallet's outputs table after the
    // 2023-10 migration that dropped MMR + range proof storage; for the purposes
    // of holding this output and being able to spend it, `None` is the correct
    // value (the new wallet reconstructs the proof when needed for spending).
    let rangeproof: Option<RangeProof> = None;

    let wallet_output = WalletOutput::new_from_parts(
        TransactionOutputVersion::get_current_version(),
        value,
        commitment_mask_key_id,
        features,
        script,
        input_data,
        script_key_id,
        sender_offset_public_key,
        metadata_signature,
        u64::try_from(row.script_lock_height).unwrap_or(0),
        covenant,
        encrypted_data,
        minimum_value_promise,
        rangeproof,
        payment_id,
        output_hash,
        commitment.clone(),
    );

    Ok(Some(ConvertedOutput {
        wallet_output,
        output_hash,
        commitment,
        value,
        mined_height: u64::try_from(mined_height).unwrap_or(0),
        mined_block_hash,
        mined_timestamp,
        received_in_tx_id: row.received_in_tx_id.map(|v| v as u64),
        spent_in_tx_id: row.spent_in_tx_id.map(|v| v as u64),
        legacy_status,
    }))
}

fn row_label(row: &ConsoleOutputRow) -> String {
    match row.received_in_tx_id {
        Some(tx) => format!("(received_in_tx_id={tx})"),
        None => "(unknown tx_id)".to_string(),
    }
}
