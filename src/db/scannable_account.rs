use crate::db::AccountRow;
use blake2::Blake2b512;
use blake2::Digest;
use blake2::digest::Update;
use tari_common_types::wallet_types::{ProvidedKeysWallet, WalletType};
use tari_crypto::ristretto::RistrettoSecretKey;
use tari_transaction_components::{
    crypto_factories::CryptoFactories,
    key_manager::{TransactionKeyManagerWrapper, memory_key_manager::MemoryKeyManagerBackend},
};
use tari_utilities::ByteArray;

use std::sync::Arc;

/// Represents an account that can be scanned for transactions
/// With Single Table Inheritance, this is simply a wrapper around AccountRow
pub(crate) struct ScannableAccount {
    account_row: AccountRow,
}

impl ScannableAccount {
    /// Create a ScannableAccount from an AccountRow
    pub fn from_account_row(account_row: AccountRow) -> Self {
        Self { account_row }
    }

    /// Get the account ID (works for both parent and child accounts)
    pub fn account_id(&self) -> Option<i64> {
        Some(self.account_row.id)
    }

    /// Get the child account ID (deprecated - all accounts now use account_id)
    /// This is kept for backward compatibility during migration
    pub fn child_account_id(&self) -> Option<i64> {
        if self.account_row.is_child() {
            Some(self.account_row.id)
        } else {
            None
        }
    }

    pub fn friendly_name(&self) -> &str {
        &self.account_row.friendly_name
    }

    pub fn birthday(&self) -> i64 {
        self.account_row.birthday
    }

    pub async fn get_key_manager(
        &self,
        password: &str,
    ) -> Result<TransactionKeyManagerWrapper<MemoryKeyManagerBackend>, anyhow::Error> {
        let (mut view_key, spend_key) = self.account_row.decrypt_keys(password)?;

        // If this is a child account, derive the tapplet-specific view key
        if self.account_row.is_child() {
            if let Some(tapplet_pub_key) = &self.account_row.tapplet_pub_key {
                let tapplet_private_view_key_bytes = Blake2b512::new()
                    .chain(b"tapplet_storage_address")
                    .chain(view_key.as_bytes())
                    .chain(hex::decode(tapplet_pub_key)?)
                    .finalize();
                view_key = RistrettoSecretKey::from_canonical_bytes(&tapplet_private_view_key_bytes)
                    .map_err(|e| anyhow::anyhow!(e))?;
            }
        }

        let wallet_type = Arc::new(WalletType::ProvidedKeys(ProvidedKeysWallet {
            view_key,
            birthday: Some(self.account_row.birthday as u16),
            public_spend_key: spend_key,
            private_spend_key: None,
            private_comms_key: None,
        }));
        let key_manager: TransactionKeyManagerWrapper<MemoryKeyManagerBackend> =
            TransactionKeyManagerWrapper::new(None, CryptoFactories::default(), wallet_type).await?;
        Ok(key_manager)
    }
}
