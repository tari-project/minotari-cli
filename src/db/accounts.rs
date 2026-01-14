use log::{debug, info, warn};
use rusqlite::{Connection, named_params};
use serde::{Deserialize, Serialize};
use serde_rusqlite::from_rows;
use tari_common::configuration::Network;
use tari_common_types::{
    seeds::{
        mnemonic::{Mnemonic, MnemonicLanguage},
        seed_words::SeedWords,
    },
    tari_address::{TariAddress, TariAddressFeatures},
    types::CompressedPublicKey,
};
use tari_crypto::{keys::PublicKey, ristretto::RistrettoPublicKey};
use tari_transaction_components::key_manager::{KeyManager, wallet_types::WalletType};
use utoipa::ToSchema;

use crate::db::error::{WalletDbError, WalletDbResult};
use crate::db::outputs::get_output_totals_for_account;
use crate::utils::{
    crypto::{decrypt_data, encrypt_data},
    fingerprint::calculate_fingerprint,
    timestamp::format_timestamp,
};
use crate::{db::balance_changes::get_balance_aggregates_for_account, utils::crypto::FullEncryptedData};
use tari_utilities::hex::Hex;

pub fn create_account(
    conn: &Connection,
    friendly_name: &str,
    wallet: &WalletType,
    password: &str,
) -> WalletDbResult<()> {
    info!(
        target: "audit",
        account = friendly_name;
        "DB: Creating new account"
    );

    let fingerprint = calculate_fingerprint(wallet);
    let birthday = wallet.get_birthday().unwrap_or(0) as i64;
    let wallet_json =
        serde_json::to_string(wallet).map_err(|e| WalletDbError::Unexpected(format!("Serialization failed: {}", e)))?;

    let encrypted_data = encrypt_data(wallet_json.as_bytes(), password)
        .map_err(|e| WalletDbError::Unexpected(format!("Encryption failed: {}", e)))?;

    conn.execute(
        r#"
        INSERT INTO accounts (
            friendly_name,
            fingerprint,
            encrypted_wallet,
            cipher_nonce,
            salt,
            birthday
        )
        VALUES (
            :name,
            :fingerprint,
            :enc_wallet,
            :nonce,
            :salt,
            :birthday
        )
        "#,
        named_params! {
            ":name": friendly_name,
            ":fingerprint": fingerprint,
            ":enc_wallet": encrypted_data.ciphertext,
            ":nonce": encrypted_data.nonce,
            ":salt": encrypted_data.salt_bytes,
            ":birthday": birthday,
        },
    )?;

    Ok(())
}

pub fn get_account_by_name(conn: &Connection, friendly_name: &str) -> WalletDbResult<Option<AccountRow>> {
    debug!(
        account = friendly_name;
        "DB: Fetching account by name"
    );

    let mut stmt = conn.prepare_cached(
        r#"
        SELECT id, 
            friendly_name, 
            fingerprint,
            encrypted_wallet,
            cipher_nonce,
            salt,
            birthday
        FROM accounts
        WHERE friendly_name = :name
        "#,
    )?;

    let rows = stmt.query(named_params! { ":name": friendly_name })?;
    let row = from_rows::<AccountRow>(rows).next().transpose()?;

    Ok(row)
}

pub fn get_accounts(conn: &Connection, friendly_name: Option<&str>) -> WalletDbResult<Vec<AccountRow>> {
    if let Some(name) = friendly_name {
        debug!(
            account = name;
            "DB: Listing accounts with filter"
        );
        let mut stmt = conn.prepare_cached(
            r#"
            SELECT id, 
              friendly_name, 
              fingerprint,
              encrypted_wallet,
              cipher_nonce,
              salt,
              birthday
            FROM accounts
            WHERE friendly_name = :name
            ORDER BY friendly_name
            "#,
        )?;
        let rows = stmt.query(named_params! { ":name": name })?;
        let results = from_rows::<AccountRow>(rows).collect::<Result<Vec<_>, _>>()?;
        Ok(results)
    } else {
        debug!("DB: Listing all accounts");
        let mut stmt = conn.prepare_cached(
            r#"
            SELECT id, 
              friendly_name, 
              fingerprint,
              encrypted_wallet,
              cipher_nonce,
              salt,
              birthday
            FROM accounts
            ORDER BY friendly_name
            "#,
        )?;
        let rows = stmt.query(named_params! {})?;
        let results = from_rows::<AccountRow>(rows).collect::<Result<Vec<_>, _>>()?;
        Ok(results)
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AccountRow {
    pub id: i64,
    pub friendly_name: String,
    pub fingerprint: Vec<u8>,
    pub encrypted_wallet: Vec<u8>,
    pub cipher_nonce: Vec<u8>,
    pub salt: Vec<u8>,
    pub birthday: i64,
}

impl AccountRow {
    pub fn decrypt_wallet_type(&self, password: &str) -> WalletDbResult<WalletType> {
        let encrypted_data = FullEncryptedData {
            ciphertext: &self.encrypted_wallet,
            nonce: &self.cipher_nonce,
            salt_bytes: &self.salt,
        };
        let plaintext_bytes = decrypt_data(&encrypted_data, password).map_err(|e| {
            warn!(error:? = e; "DB: Failed to decrypt wallet");
            WalletDbError::DecryptionFailed("Failed to decrypt wallet data".to_string())
        })?;

        let wallet: WalletType = serde_json::from_slice(&plaintext_bytes)
            .map_err(|e| WalletDbError::Decoding(format!("Failed to deserialize wallet JSON: {}", e)))?;

        Ok(wallet)
    }

    pub fn get_address(&self, network: Network, password: &str) -> WalletDbResult<TariAddress> {
        let wallet = self.decrypt_wallet_type(password)?;

        let view_private_key = wallet.get_view_key();
        let spend_public_key = wallet.get_public_spend_key();

        let view_public_key = RistrettoPublicKey::from_secret_key(view_private_key);
        let view_public_compressed = CompressedPublicKey::new_from_pk(view_public_key);

        let address = TariAddress::new_dual_address(
            view_public_compressed,
            spend_public_key,
            network,
            TariAddressFeatures::create_one_sided_only(),
            None,
        )
        .map_err(|e| WalletDbError::Unexpected(format!("Failed to generate address: {}", e)))?;

        Ok(address)
    }

    pub fn get_key_manager(&self, password: &str) -> WalletDbResult<KeyManager> {
        let wallet = self.decrypt_wallet_type(password)?;
        let key_manager = KeyManager::new(wallet)
            .map_err(|e| WalletDbError::Unexpected(format!("Failed to create key manager: {}", e)))?;

        Ok(key_manager)
    }

    pub fn get_seed_words(&self, password: &str) -> WalletDbResult<Option<SeedWords>> {
        let wallet = self.decrypt_wallet_type(password)?;

        match wallet {
            WalletType::SeedWords(seed_wallet) => {
                let cipher_seed = seed_wallet.cipher_seed();
                let mnemonic = cipher_seed.to_mnemonic(MnemonicLanguage::English, None).map_err(|e| {
                    warn!(error:? = e; "DB: Failed to convert seed to mnemonic");
                    WalletDbError::Unexpected(format!("Failed to generate mnemonic: {}", e))
                })?;
                Ok(Some(mnemonic))
            },
            _ => Ok(None),
        }
    }

    pub fn get_keys_hex(&self, password: &str) -> WalletDbResult<(String, String)> {
        let wallet = self.decrypt_wallet_type(password)?;

        let view_key = wallet.get_view_key();
        let spend_key = wallet.get_public_spend_key();

        Ok((view_key.to_hex(), spend_key.to_hex()))
    }
}

#[derive(Debug, Clone, ToSchema, Serialize)]
pub struct AccountBalance {
    /// The total balance of the account (Total Credits - Total Debits).
    pub total: u64,
    /// The portion of the total balance that is currently spendable.
    pub available: u64,
    /// The portion of the balance that is locked.
    pub locked: u64,
    /// The amount from incoming transactions that have not yet been confirmed.
    pub unconfirmed: u64,
    /// The total sum of all incoming (credit) transactions.
    pub total_credits: Option<i64>,
    /// The total sum of all outgoing (debit) transactions.
    pub total_debits: Option<i64>,
    /// The maximum blockchain height among all transactions for this account.
    ///
    /// Will be `None` if the account has no transactions.
    pub max_height: Option<i64>,
    /// The timestamp of the most recent transaction.
    ///
    /// The string is in ISO 8601 format. Will be `None` if the
    /// account has no transactions.
    pub max_date: Option<String>,
}

pub fn get_balance(conn: &Connection, account_id: i64) -> WalletDbResult<AccountBalance> {
    debug!(
        account_id = account_id;
        "DB: Calculating account balance"
    );
    let history_agg = get_balance_aggregates_for_account(conn, account_id)?;
    let (locked_amount, unconfirmed_amount, locked_and_unconfirmed_amount) =
        get_output_totals_for_account(conn, account_id)?;

    let total_credits = history_agg.total_credits.unwrap_or(0) as u64;
    let total_debits = history_agg.total_debits.unwrap_or(0) as u64;
    let total_balance = total_credits.saturating_sub(total_debits);

    let unavailable_balance = locked_amount
        .saturating_add(unconfirmed_amount)
        .saturating_sub(locked_and_unconfirmed_amount);
    let available_balance = total_balance.saturating_sub(unavailable_balance);

    let max_date_str = history_agg.max_date.map(format_timestamp);

    Ok(AccountBalance {
        total: total_balance,
        available: available_balance,
        locked: locked_amount,
        unconfirmed: unconfirmed_amount,
        total_credits: history_agg.total_credits,
        total_debits: history_agg.total_debits,
        max_height: history_agg.max_height,
        max_date: max_date_str,
    })
}
