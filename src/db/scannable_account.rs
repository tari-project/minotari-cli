use crate::db::{AccountRow, accounts::ChildAccountRow};
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
pub(crate) struct ScannableAccount {
    parent_account_row: AccountRow,
    child_account_row: Option<ChildAccountRow>,
}

impl ScannableAccount {
    pub fn account_id(&self) -> Option<i64> {
        if self.child_account_row.is_none() {
            return Some(self.parent_account_row.id);
        } else {
            None
        }
    }

    pub fn child_account_id(&self) -> Option<i64> {
        self.child_account_row.as_ref().map(|c| c.id)
    }

    pub fn friendly_name(&self) -> &str {
        if let Some(child_account) = &self.child_account_row {
            &child_account.child_account_name
        } else {
            &self.parent_account_row.friendly_name
        }
    }

    pub fn birthday(&self) -> i64 {
        self.parent_account_row.birthday
    }

    pub async fn get_key_manager(
        &self,
        password: &str,
    ) -> Result<TransactionKeyManagerWrapper<MemoryKeyManagerBackend>, anyhow::Error> {
        let (mut view_key, spend_key) = self.parent_account_row.decrypt_keys(password)?;

        if let Some(child_account) = &self.child_account_row {
            let tapplet_private_view_key_bytes = Blake2b512::new()
                .chain(b"tapplet_storage_address")
                .chain(view_key.as_bytes())
                .chain(hex::decode(&child_account.tapplet_pub_key)?)
                .finalize();
            view_key = RistrettoSecretKey::from_canonical_bytes(&tapplet_private_view_key_bytes)
                .map_err(|e| anyhow::anyhow!(e))?;
        }
        let wallet_type = Arc::new(WalletType::ProvidedKeys(ProvidedKeysWallet {
            view_key,
            birthday: Some(self.parent_account_row.birthday as u16), // TODO: Allow child accounts to have their own birthday
            public_spend_key: spend_key,
            private_spend_key: None,
            private_comms_key: None,
        }));
        let key_manager: TransactionKeyManagerWrapper<MemoryKeyManagerBackend> =
            TransactionKeyManagerWrapper::new(None, CryptoFactories::default(), wallet_type).await?;
        Ok(key_manager)
    }
}
