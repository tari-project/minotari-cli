use tari_common_types::types::CompressedPublicKey;
use tari_crypto::ristretto::RistrettoSecretKey;
use tari_transaction_components::key_manager::{
    KeyManager,
    wallet_types::{ViewWallet, WalletType},
};

#[derive(Debug, Clone, Default)]
pub struct KeyManagerBuilder {
    wallet_type: Option<WalletType>,
}

impl KeyManagerBuilder {
    #[must_use]
    pub fn with_view_key_and_spend_key(
        mut self,
        view_key: RistrettoSecretKey,
        spend_key: CompressedPublicKey,
        birthday: u16,
    ) -> Self {
        let wallet = ViewWallet::new(spend_key, view_key, Some(birthday));
        self.wallet_type = Some(WalletType::ViewWallet(wallet));
        self
    }

    pub fn try_build(self) -> Result<KeyManager, anyhow::Error> {
        if let Some(wallet_type) = self.wallet_type {
            Ok(KeyManager::new(wallet_type)?)
        } else {
            Ok(KeyManager::new_random()?)
        }
    }
}
