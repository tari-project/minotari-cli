use chacha20poly1305::{Key, KeyInit, XChaCha20Poly1305, XNonce, aead::Aead};
use log::{debug, info, warn};
use rusqlite::{Connection, named_params};
use serde::{Deserialize, Serialize};
use serde_rusqlite::from_rows;
use tari_common::configuration::Network;
use tari_common_types::{
    tari_address::{TariAddress, TariAddressFeatures},
    types::CompressedPublicKey,
};
use tari_crypto::keys::PublicKey;
use tari_crypto::{
    compressed_key::CompressedKey,
    ristretto::{RistrettoPublicKey, RistrettoSecretKey},
};
use tari_transaction_components::key_manager::KeyManager;
use tari_transaction_components::key_manager::wallet_types::ViewWallet;
use tari_transaction_components::key_manager::wallet_types::WalletType;

use tari_utilities::byte_array::ByteArray;
use utoipa::ToSchema;

use crate::db::balance_changes::get_balance_aggregates_for_account;
use crate::db::error::{WalletDbError, WalletDbResult};
use crate::db::outputs::get_output_totals_for_account;
use crate::utils::format_timestamp;

pub fn create_account(
    conn: &Connection,
    friendly_name: &str,
    encrypted_view_private_key: &[u8],
    encrypted_spend_public_key: &[u8],
    cipher_nonce: &[u8],
    unencrypted_view_key_hash: &[u8],
    birthday: i64,
) -> WalletDbResult<()> {
    info!(
        target: "audit",
        account = friendly_name;
        "DB: Creating new account"
    );

    conn.execute(
        r#"
        INSERT INTO accounts (
            friendly_name,
            encrypted_view_private_key,
            encrypted_spend_public_key,
            cipher_nonce,
            unencrypted_view_key_hash,
            birthday
        )
        VALUES (
            :name,
            :enc_view,
            :enc_spend,
            :nonce,
            :view_hash,
            :birthday
        )
        "#,
        named_params! {
            ":name": friendly_name,
            ":enc_view": encrypted_view_private_key,
            ":enc_spend": encrypted_spend_public_key,
            ":nonce": cipher_nonce,
            ":view_hash": unencrypted_view_key_hash,
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
            encrypted_view_private_key, 
            encrypted_spend_public_key, 
            cipher_nonce, 
            unencrypted_view_key_hash,
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
              encrypted_view_private_key, 
              encrypted_spend_public_key, 
              cipher_nonce, 
              unencrypted_view_key_hash,
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
              encrypted_view_private_key, 
              encrypted_spend_public_key, 
              cipher_nonce, 
              unencrypted_view_key_hash,
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
    pub encrypted_view_private_key: Vec<u8>,
    pub encrypted_spend_public_key: Vec<u8>,
    pub cipher_nonce: Vec<u8>,
    #[allow(dead_code)]
    pub unencrypted_view_key_hash: Option<Vec<u8>>,
    pub birthday: i64,
}

impl AccountRow {
    pub fn decrypt_keys(
        &self,
        password: &str,
    ) -> WalletDbResult<(RistrettoSecretKey, CompressedKey<RistrettoPublicKey>)> {
        let password = if password.len() < 32 {
            format!("{:0<32}", password)
        } else {
            password[..32].to_string()
        };

        let key_bytes: [u8; 32] = password
            .as_bytes()
            .try_into()
            .map_err(|_| WalletDbError::DecryptionFailed("Password conversion failed".to_string()))?;

        let key = Key::from(key_bytes);
        let cipher = XChaCha20Poly1305::new(&key);

        let nonce_bytes: &[u8; 24] = self
            .cipher_nonce
            .as_slice()
            .try_into()
            .map_err(|_| WalletDbError::DecryptionFailed("Nonce must be 24 bytes".to_string()))?;

        let nonce = XNonce::from(*nonce_bytes);

        let view_key = cipher
            .decrypt(&nonce, self.encrypted_view_private_key.as_ref())
            .map_err(|e| {
                warn!(error:? = e; "DB: Failed to decrypt view key");
                WalletDbError::DecryptionFailed("Failed to decrypt view key".to_string())
            })?;

        let spend_key = cipher
            .decrypt(&nonce, self.encrypted_spend_public_key.as_ref())
            .map_err(|e| {
                warn!(error:? = e; "DB: Failed to decrypt spend key");
                WalletDbError::DecryptionFailed("Failed to decrypt spend key".to_string())
            })?;

        let view_key = RistrettoSecretKey::from_canonical_bytes(&view_key)
            .map_err(|e| WalletDbError::DecryptionFailed(format!("Invalid view key bytes: {}", e)))?;

        let spend_key = CompressedKey::<RistrettoPublicKey>::from_canonical_bytes(&spend_key)
            .map_err(|e| WalletDbError::DecryptionFailed(format!("Invalid spend key bytes: {}", e)))?;

        Ok((view_key, spend_key))
    }

    pub fn get_address(&self, network: Network, password: &str) -> WalletDbResult<TariAddress> {
        let (view_key, spend_key) = self.decrypt_keys(password)?;
        let address = TariAddress::new_dual_address(
            CompressedPublicKey::new_from_pk(RistrettoPublicKey::from_secret_key(&view_key)),
            spend_key,
            network,
            TariAddressFeatures::create_one_sided_only(),
            None,
        )
        .map_err(|e| WalletDbError::Unexpected(format!("Failed to generate address: {}", e)))?;

        Ok(address)
    }

    pub fn get_key_manager(&self, password: &str) -> WalletDbResult<KeyManager> {
        let (view_key, spend_key) = self.decrypt_keys(password)?;
        let view_wallet = ViewWallet::new(spend_key, view_key, Some(self.birthday as u16));
        let wallet_type = WalletType::ViewWallet(view_wallet);
        let key_manager = KeyManager::new(wallet_type)
            .map_err(|e| WalletDbError::Unexpected(format!("Failed to create key manager: {}", e)))?;

        Ok(key_manager)
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
