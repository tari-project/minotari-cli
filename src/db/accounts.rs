use chacha20poly1305::{Key, KeyInit, XChaCha20Poly1305, XNonce, aead::Aead};
use serde::Serialize;
use sqlx::SqliteConnection;
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
use zeroize::Zeroizing;

use tari_utilities::byte_array::ByteArray;
use utoipa::ToSchema;

use crate::db::balance_changes::get_balance_aggregates_for_account;
use crate::db::outputs::get_output_totals_for_account;

pub async fn create_account(
    conn: &mut SqliteConnection,
    friendly_name: &str,
    encryptd_view_private_key: &[u8],
    encrypted_spend_public_key: &[u8],
    cipher_nonce: &[u8],
    unencrypted_view_key_hash: &[u8],
    birthday: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        r#"
        INSERT INTO accounts (friendly_name, 
          encrypted_view_private_key, 
          encrypted_spend_public_key, 
          cipher_nonce, 
          unencrypted_view_key_hash,
          birthday)
        VALUES (?, ?, ?, ?, ?, ?)
        "#,
        friendly_name,
        encryptd_view_private_key,
        encrypted_spend_public_key,
        cipher_nonce,
        unencrypted_view_key_hash,
        birthday
    )
    .execute(&mut *conn)
    .await?;

    Ok(())
}

pub async fn get_account_by_name(
    conn: &mut SqliteConnection,
    friendly_name: &str,
) -> Result<Option<AccountRow>, sqlx::Error> {
    let row = sqlx::query_as!(
        AccountRow,
        r#"
        SELECT id, 
            friendly_name, 
            encrypted_view_private_key, 
            encrypted_spend_public_key, 
            cipher_nonce, 
            unencrypted_view_key_hash,
            birthday
        FROM accounts
        WHERE friendly_name = ?
        "#,
        friendly_name
    )
    .fetch_optional(&mut *conn)
    .await?;

    Ok(row)
}

pub async fn get_accounts(
    conn: &mut SqliteConnection,
    friendly_name: Option<&str>,
) -> Result<Vec<AccountRow>, sqlx::Error> {
    let rows = if let Some(name) = friendly_name {
        sqlx::query_as!(
            AccountRow,
            r#"
            SELECT id, 
              friendly_name, 
              encrypted_view_private_key, 
              encrypted_spend_public_key, 
              cipher_nonce, 
              unencrypted_view_key_hash,
              birthday
            FROM accounts
            WHERE friendly_name = ?
            ORDER BY friendly_name
            "#,
            name
        )
        .fetch_all(&mut *conn)
        .await?
    } else {
        sqlx::query_as!(
            AccountRow,
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
            "#
        )
        .fetch_all(&mut *conn)
        .await?
    };

    Ok(rows)
}

#[derive(sqlx::FromRow, Debug)]
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
        password: &Zeroizing<String>,
    ) -> Result<(RistrettoSecretKey, CompressedKey<RistrettoPublicKey>), anyhow::Error> {
        let password = Zeroizing::new(if password.len() < 32 {
            format!("{:0<32}", password.as_str())
        } else {
            if password.len() > 32 {
                return Err(anyhow::anyhow!("Password must be at most 32 bytes"));
            } else {
                password[..].to_string()
            }
        });
        let key_bytes: [u8; 32] = password
            .as_bytes()
            .try_into()
            .map_err(|_| anyhow::anyhow!("Password must be 32 bytes"))?;
        let key = Key::from(key_bytes);
        let cipher = XChaCha20Poly1305::new(&key);

        let nonce_bytes: &[u8; 24] = self
            .cipher_nonce
            .as_slice()
            .try_into()
            .map_err(|_| anyhow::anyhow!("Nonce must be 24 bytes"))?;
        let nonce = XNonce::from(*nonce_bytes);

        let view_key = cipher.decrypt(&nonce, self.encrypted_view_private_key.as_ref())?;
        let spend_key = cipher.decrypt(&nonce, self.encrypted_spend_public_key.as_ref())?;

        let view_key = RistrettoSecretKey::from_canonical_bytes(&view_key).map_err(|e| anyhow::anyhow!(e))?;
        let spend_key =
            CompressedKey::<RistrettoPublicKey>::from_canonical_bytes(&spend_key).map_err(|e| anyhow::anyhow!(e))?;
        Ok((view_key, spend_key))
    }

    pub fn get_address(&self, network: Network, password: &Zeroizing<String>) -> Result<TariAddress, anyhow::Error> {
        let (view_key, spend_key) = self.decrypt_keys(password)?;
        let address = TariAddress::new_dual_address(
            CompressedPublicKey::new_from_pk(RistrettoPublicKey::from_secret_key(&view_key)),
            spend_key,
            network,
            TariAddressFeatures::create_one_sided_only(),
            None,
        )?;
        Ok(address)
    }

    pub async fn get_key_manager(&self, password: &Zeroizing<String>) -> Result<KeyManager, anyhow::Error> {
        let (view_key, spend_key) = self.decrypt_keys(password)?;
        let view_wallet = ViewWallet::new(spend_key, view_key, Some(self.birthday as u16));
        let wallet_type = WalletType::ViewWallet(view_wallet);
        let key_manager = KeyManager::new(wallet_type)?;
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

pub async fn get_balance(conn: &mut SqliteConnection, account_id: i64) -> Result<AccountBalance, sqlx::Error> {
    let history_agg = get_balance_aggregates_for_account(conn, account_id).await?;
    let (locked_amount, unconfirmed_amount, locked_and_unconfirmed_amount) =
        get_output_totals_for_account(conn, account_id).await?;
    let total_credits = history_agg.total_credits.unwrap_or(0) as u64;
    let total_debits = history_agg.total_debits.unwrap_or(0) as u64;
    let total_balance = total_credits.saturating_sub(total_debits);
    let unavailable_balance = locked_amount
        .saturating_add(unconfirmed_amount)
        .saturating_sub(locked_and_unconfirmed_amount);
    let available_balance = total_balance.saturating_sub(unavailable_balance);

    Ok(AccountBalance {
        total: total_balance,
        available: available_balance,
        locked: locked_amount,
        unconfirmed: unconfirmed_amount,
        total_credits: history_agg.total_credits,
        total_debits: history_agg.total_debits,
        max_height: history_agg.max_height,
        max_date: history_agg.max_date,
    })
}
