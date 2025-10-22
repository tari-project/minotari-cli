use std::{fs, path::PathBuf, sync::Arc};

use crate::{
    db::{AccountTypeRow, get_child_account},
    get_accounts, init_db,
    transactions::one_sided_transaction::{OneSidedTransaction, Recipient},
};
use anyhow::anyhow;
use async_trait::async_trait;
use blake2::{Blake2b512, Blake2s, Digest, digest::Update};
use regex::Regex;
use sqlx::{SqliteConnection, SqlitePool};
use tari_common::configuration::Network;
use tari_common_types::tari_address::{TariAddress, TariAddressFeatures};
use tari_crypto::{
    compressed_key::CompressedKey,
    keys::SecretKey,
    ristretto::{RistrettoPublicKey, RistrettoSecretKey},
};
use tari_tapplet_lib::{TappletConfig, host::MinotariTappletApiV1};
use tari_transaction_components::MicroMinotari;
use tari_utilities::ByteArray;

#[derive(Clone)]
pub(crate) struct MinotariApiProvider {
    account_name: String,
    private_view_key: RistrettoSecretKey,
    public_spend_key: CompressedKey<RistrettoPublicKey>,
    account_id: i64,
    child_account_id: i64,
    tapplet_name: String,
    tapplet_public_key: CompressedKey<RistrettoPublicKey>,
    default_amount_for_save: MicroMinotari,
    seconds_to_lock_utxos: u64,
    db_file: PathBuf,
    password: String,
}

fn derive_tapplet_account_name(account_name: &str, tapplet_canonical_name: &str) -> String {
    format!("{}::{}", account_name, tapplet_canonical_name)
}

impl MinotariApiProvider {
    pub async fn try_create(
        account_name: String,
        tapplet_config: &TappletConfig,
        database_file: PathBuf,
        password: String,
    ) -> Result<Self, anyhow::Error> {
        let pool = init_db(&database_file).await?;
        let mut conn = pool.acquire().await?;
        // let tapplet_account = derive_tapplet_account_name(&account_name, &tapplet_config.canonical_name());
        let accounts = get_accounts(&mut conn, Some(&account_name), false).await?;

        if accounts.is_empty() {
            Err(anyhow::anyhow!("No account found with name '{}'", account_name))
        } else if accounts.len() > 1 {
            println!(
                "Multiple accounts found with name '{}'. Please ensure only one account per tapplet.",
                account_name
            );
            Err(anyhow::anyhow!("Multiple accounts found"))
        } else {
            let account = &accounts[0];
            println!("Found account: {:?}", account);
            let account_type_row = AccountTypeRow::try_from_account(&mut conn, account.clone()).await?;

            let (view_key, spend_key) = account_type_row.decrypt_keys(&password)?;

            let child_account = get_child_account(&mut conn, account.id, &tapplet_config.name).await?;

            let child_pub_key = CompressedKey::<RistrettoPublicKey>::from_canonical_bytes(&hex::decode(
                child_account
                    .tapplet_pub_key
                    .as_ref()
                    .ok_or_else(|| anyhow!("Child account missing tapplet_pub_key"))?,
            )?)
            .map_err(|e| anyhow!("Could not decode public key"))?;
            Ok(Self {
                account_name,
                private_view_key: view_key,
                public_spend_key: spend_key,
                account_id: account.id,
                child_account_id: child_account.id,
                tapplet_name: tapplet_config.canonical_name(),
                tapplet_public_key: child_pub_key,
                default_amount_for_save: MicroMinotari::from(1_000), // 0.001 Minotari
                seconds_to_lock_utxos: 30,
                db_file: database_file,
                password,
            })
        }
    }
}

#[async_trait]
impl MinotariTappletApiV1 for MinotariApiProvider {
    async fn append_data(&self, slot: &str, value: &str) -> Result<(), anyhow::Error> {
        println!("Appending data to slot '{}': {}", slot, value);

        let tapplet_private_view_key_bytes = Blake2b512::new()
            .chain(b"tapplet_storage_address")
            .chain(self.private_view_key.as_bytes())
            .chain(self.tapplet_public_key.as_bytes())
            .finalize();
        let tapplet_private_view_key = RistrettoSecretKey::from_uniform_bytes(&tapplet_private_view_key_bytes)
            .map_err(|e| anyhow::anyhow!("Failed to create tapplet private view key: {}", e))?;

        let tapplet_view_pub_key = CompressedKey::<RistrettoPublicKey>::from_secret_key(&tapplet_private_view_key);
        let spend_key = self.public_spend_key.clone();
        let tapplet_storage_address = TariAddress::new_dual_address(
            tapplet_view_pub_key,
            spend_key,
            Network::MainNet,
            TariAddressFeatures::create_one_sided_only(),
            None,
        )?;

        let recipients = vec![Recipient {
            address: tapplet_storage_address.clone(),
            amount: self.default_amount_for_save,
        }];
        let payment_id = format!(
            "t:\"{}\",\"{}\"",
            slot.replace("\"", "\\\""),
            value.replace("\"", "\\\"")
        );
        println!(
            "You can send a manual transaction with this payment memo: {} to address {}",
            payment_id,
            tapplet_storage_address.to_base58()
        );
        println!("Creating unsigned one-sided transaction. If this fails, use the above fallback ...");

        let seconds_to_lock_utxos = self.seconds_to_lock_utxos;
        let path = self.db_file.to_string_lossy();
        let db = SqlitePool::connect(&path).await?;
        let mut conn = db.acquire().await?;
        let account = crate::db::get_account_by_name(&mut conn, &self.account_name)
            .await?
            .ok_or_else(|| anyhow!("No account found. This should not happen."))?;

        let parent_account = account
            .try_into_parent()
            .map_err(|e| anyhow::anyhow!("Invalid account type: {}", e))?;
        let one_sided_tx = OneSidedTransaction::new(db.clone(), Network::MainNet, self.password.clone());

        let result = one_sided_tx
            .create_unsigned_transaction(
                parent_account,
                recipients,
                Some(payment_id.clone()),
                Some("tapplet_append_data".to_string()),
                seconds_to_lock_utxos,
            )
            .await?;

        fs::write("unsigned_one_sided_tx.json", serde_json::to_string_pretty(&result)?)?;
        println!(
            "Unsigned one-sided transaction written to 'unsigned_one_sided_tx.json'. Sign this and send to the network."
        );
        println!(
            "Otherwise send a manual transaction with this payment memo: {} to address {}",
            payment_id,
            tapplet_storage_address.to_base58()
        );

        dbg!(&result);

        Ok(())
    }

    async fn load_data_entries(&self, slot: &str) -> Result<Vec<String>, anyhow::Error> {
        println!("Loading data entries from slot '{}'", slot);
        // Read outputs for this account from the database
        let path = self.db_file.to_string_lossy();
        let db = SqlitePool::connect(&path).await?;
        let mut conn = db.acquire().await?;

        dbg!(self.child_account_id);
        // TODO: Pagination
        let outputs = crate::db::get_output_memos_for_account(&mut conn, self.child_account_id, 100, 0).await?;

        dbg!(&outputs);
        let mut entries = Vec::new();
        let pattern = format!(r#"^t:"{}","(.+)"$"#, regex::escape(slot));
        let re = Regex::new(&pattern)?;
        for (id, _memo_hex, memo_parsed) in outputs {
            dbg!(&id, &memo_parsed);
            if let Some(captures) = re.captures(&memo_parsed) {
                if let Some(value) = captures.get(1) {
                    entries.push(value.as_str().to_string());
                }
            }
        }
        dbg!(&entries);

        Ok(entries)
    }
}
