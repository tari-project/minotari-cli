use serde::{Deserialize, Serialize};
use tari_common_types::types::{CompressedCommitment, CompressedPublicKey, FixedHash};
use tari_crypto::compressed_key::CompressedKey;
use tari_transaction_components::{
    MicroMinotari,
    key_manager::{TariKeyId, TransactionKeyManagerInterface},
    rpc::models::MinimalUtxoSyncInfo,
    transaction_components::{EncryptedData, MemoField, TransactionOutput, WalletOutput},
};
use tari_utilities::ByteArray;

use crate::WalletError;

/// HTTP API tip info response - matches the actual API structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpTipInfoResponse {
    pub metadata: HttpChainMetadata,
}

/// HTTP API chain metadata structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpChainMetadata {
    pub best_block_height: u64,
    pub best_block_hash: Vec<u8>,
    pub accumulated_difficulty: String,
    pub pruned_height: u64,
    pub timestamp: u64,
}

/// HTTP API block header response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpHeaderResponse {
    pub hash: Vec<u8>,
    pub height: u64,
    pub timestamp: u64,
}

/// HTTP API single block response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SingleBlockResponse {
    pub block: HttpBlock,
}

/// HTTP API block structure  
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpBlock {
    pub header: HttpBlockHeader,
    pub body: HttpBlockBody,
}

/// HTTP API block header structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpBlockHeader {
    pub height: u64,
    pub hash: Vec<u8>,
    pub timestamp: u64,
}

/// HTTP API block body structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpBlockBody {
    pub outputs: Vec<TransactionOutput>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncompleteScannedOutput {
    pub output_hash: FixedHash,
    pub value: MicroMinotari,
    pub commitment_mask_key_id: TariKeyId,
    pub sender_offset_public_key: CompressedPublicKey,
    pub encrypted_data: EncryptedData,
    pub memo: MemoField,
    pub key_manager_index: usize,
}

impl IncompleteScannedOutput {
    pub fn new(
        scanning_info: &ScanningOutputStruct,
        value: MicroMinotari,
        commitment_mask_key_id: TariKeyId,
        memo: MemoField,
        index: usize,
    ) -> Result<Self, WalletError> {
        let output_hash = FixedHash::try_from(scanning_info.min_info.output_hash.clone())
            .map_err(|e| WalletError::DataError(e.to_string()))?;
        Ok(Self {
            output_hash,
            value,
            commitment_mask_key_id,
            sender_offset_public_key: scanning_info.sender_offset_public_key.clone(),
            encrypted_data: scanning_info.encrypted_data.clone(),
            memo,
            key_manager_index: index,
        })
    }

    pub fn to_wallet_output<KM: TransactionKeyManagerInterface>(
        &self,
        output: TransactionOutput,
        key_manager: &KM,
    ) -> Result<Option<WalletOutput>, WalletError> {
        match WalletOutput::new_imported(
            self.value,
            self.commitment_mask_key_id.clone(),
            self.memo.clone(),
            output,
            key_manager,
        ) {
            Ok(wo) => Ok(Some(wo)),
            Err(_e) => Ok(None),
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ScanningOutputStruct {
    pub min_info: MinimalUtxoSyncInfo,
    pub commitment: CompressedCommitment,
    pub encrypted_data: EncryptedData,
    pub sender_offset_public_key: CompressedPublicKey,
}

impl TryFrom<MinimalUtxoSyncInfo> for ScanningOutputStruct {
    type Error = WalletError;

    fn try_from(mim_info: MinimalUtxoSyncInfo) -> Result<Self, Self::Error> {
        let commitment = CompressedCommitment::from_canonical_bytes(&mim_info.commitment)
            .map_err(|e| WalletError::DataError(e.to_string()))?;
        let encrypted_data =
            EncryptedData::from_bytes(&mim_info.encrypted_data).map_err(|e| WalletError::DataError(e.to_string()))?;
        let sender_offset_public_key = CompressedKey::from_canonical_bytes(&mim_info.sender_offset_public_key)
            .map_err(|e| WalletError::DataError(e.to_string()))?;
        Ok(Self {
            min_info: mim_info,
            commitment,
            encrypted_data,
            sender_offset_public_key,
        })
    }
}
