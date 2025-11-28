use chacha20poly1305::{Key, KeyInit, XChaCha20Poly1305, XNonce, aead::Aead};
use serde::Serialize;
use sqlx::SqliteConnection;
use std::sync::LazyLock;
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
use tokio::sync::{
    Mutex,
    mpsc::{Receiver, Sender},
};
use utoipa::ToSchema;

pub static ACCOUNT_CREATION_CHANNEL: LazyLock<(Sender<AccountRow>, Mutex<Receiver<AccountRow>>)> =
    LazyLock::new(|| {
        let (tx, rx) = tokio::sync::mpsc::channel(100);
        (tx, Mutex::new(rx))
    });

pub async fn create_account(
    conn: &mut SqliteConnection,
    friendly_name: &str,
    encryptd_view_private_key: &[u8],
    encrypted_spend_public_key: &[u8],
    cipher_nonce: &[u8],
    unencrypted_view_key_hash: &[u8],
    birthday: i64,
) -> Result<(), sqlx::Error> {
    let query_result = sqlx::query!(
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

    // Notify listeners about the new account
    // Do not fail because of notification failure
    let _unused = ACCOUNT_CREATION_CHANNEL
        .0
        .send(AccountRow {
            id: query_result.last_insert_rowid(),
            friendly_name: friendly_name.to_string(),
            encrypted_view_private_key: encryptd_view_private_key.to_vec(),
            encrypted_spend_public_key: encrypted_spend_public_key.to_vec(),
            cipher_nonce: cipher_nonce.to_vec(),
            unencrypted_view_key_hash: Some(unencrypted_view_key_hash.to_vec()),
            birthday,
        })
        .await
        .map_err(|e| sqlx::Error::Protocol(e.to_string()));

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
        password: &str,
    ) -> Result<(RistrettoSecretKey, CompressedKey<RistrettoPublicKey>), anyhow::Error> {
        let password = if password.len() < 32 {
            format!("{:0<32}", password)
        } else {
            password[..32].to_string()
        };
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

    pub fn get_address(&self, network: Network, password: &str) -> Result<TariAddress, anyhow::Error> {
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

    pub async fn get_key_manager(&self, password: &str) -> Result<KeyManager, anyhow::Error> {
        let (view_key, spend_key) = self.decrypt_keys(password)?;
        let view_wallet = ViewWallet::new(spend_key, view_key, Some(self.birthday as u16));
        let wallet_type = WalletType::ViewWallet(view_wallet);
        let key_manager = KeyManager::new(wallet_type)?;
        Ok(key_manager)
    }
}

#[derive(Debug, Clone, ToSchema, Serialize)]
pub struct AccountBalance {
    pub total_credits: Option<i64>,
    pub total_debits: Option<i64>,
    pub max_height: Option<i64>,
    pub max_date: Option<String>,
}

pub async fn get_balance(conn: &mut SqliteConnection, account_id: i64) -> Result<AccountBalance, sqlx::Error> {
    let agg_result = sqlx::query_as!(
        AccountBalance,
        r#"
            SELECT 
              SUM(balance_credit) as "total_credits: _",
              Sum(balance_debit) as "total_debits: _",
              max(effective_height) as "max_height: _",
              strftime('%Y-%m-%d %H:%M:%S', max(effective_date))  as "max_date: _"
            FROM balance_changes
            WHERE account_id = ?
            "#,
        account_id
    )
    .fetch_one(&mut *conn)
    .await?;
    Ok(agg_result)
}
