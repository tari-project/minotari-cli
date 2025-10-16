use tari_crypto::{
    compressed_key::CompressedKey,
    ristretto::{RistrettoPublicKey, RistrettoSecretKey},
};
use tari_tapplet_lib::{TappletConfig, host::MinotariTappletApiV1};

use crate::{get_accounts, init_db, scan::decrypt_keys};

#[derive(Clone)]
pub(crate) struct MinotariApiProvider {
    account_name: String,
    private_view_key: RistrettoSecretKey,
    public_spend_key: CompressedKey<RistrettoPublicKey>,
}

fn derive_tapplet_account_name(account_name: &str, tapplet_canonical_name: &str) -> String {
    format!("{}::{}", account_name, tapplet_canonical_name)
}

impl MinotariApiProvider {
    pub async fn try_create(
        account_name: String,
        tapplet_config: &TappletConfig,
        database_file: &str,
        password: &str,
    ) -> Result<Self, anyhow::Error> {
        let db = init_db(database_file).await?;
        let tapplet_account = derive_tapplet_account_name(&account_name, &tapplet_config.canonical_name());
        let accounts = get_accounts(&db, Some(&tapplet_account)).await?;
        if accounts.is_empty() {
            Err(anyhow::anyhow!("No account found with name '{}'", tapplet_account))
        } else if accounts.len() > 1 {
            println!(
                "Multiple accounts found with name '{}'. Please ensure only one account per tapplet.",
                tapplet_account
            );
            Err(anyhow::anyhow!("Multiple accounts found"))
        } else {
            let account = &accounts[0];
            println!("Found account: {:?}", account);
            let (view_key, spend_key) = decrypt_keys(&account, password)?;
            Ok(Self {
                account_name: tapplet_account,
                private_view_key: view_key,
                public_spend_key: spend_key,
            })
        }
    }
}

impl MinotariTappletApiV1 for MinotariApiProvider {
    fn append_data(&self, slot: &str, value: &str) -> Result<(), anyhow::Error> {
        println!("Appending data to slot '{}': {}", slot, value);
        Ok(())
    }

    fn load_data_entries(&self, slot: &str) -> Result<Vec<String>, anyhow::Error> {
        println!("Loading data entries from slot '{}'", slot);
        Ok(vec!["example_entry_1".to_string(), "example_entry_2".to_string()])
    }
}
