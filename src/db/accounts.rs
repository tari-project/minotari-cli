use std::sync::Arc;

use crate::db::scannable_account::ScannableAccount;
use crate::models::Id;
use blake2::Blake2b512;
use blake2::Digest;
use blake2::digest::Update;
use chacha20poly1305::{Key, KeyInit, XChaCha20Poly1305, XNonce, aead::Aead};
use serde::Serialize;
use sqlx::{SqliteConnection, SqlitePool};
use tari_common::configuration::Network;
use tari_common_types::{
    seeds::cipher_seed::CipherSeed,
    tari_address::{TariAddress, TariAddressFeatures},
    types::CompressedPublicKey,
    wallet_types::{ProvidedKeysWallet, WalletType},
};
use tari_crypto::keys::PublicKey;
use tari_crypto::{
    compressed_key::CompressedKey,
    ristretto::{RistrettoPublicKey, RistrettoSecretKey},
};
use tari_transaction_components::{
    crypto_factories::CryptoFactories,
    key_manager::{TransactionKeyManagerWrapper, memory_key_manager::MemoryKeyManagerBackend},
};
use tari_utilities::byte_array::ByteArray;
use utoipa::ToSchema;

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

pub async fn get_scannable_accounts(
    conn: &mut SqliteConnection,
    friendly_name: Option<&str>,
    include_child_accounts: bool,
) -> Result<Vec<ScannableAccount>, sqlx::Error> {
    let accounts = get_accounts(conn, friendly_name).await?;
    let mut scannable_accounts = Vec::new();
    for account in accounts {
        scannable_accounts.push(ScannableAccount {
            parent_account_row: account.clone(),
            child_account_row: None,
        });
        if include_child_accounts {
            // Include child accounts
            let child_accounts = sqlx::query_as!(
                ChildAccountRow,
                r#"
                SELECT id, 
                    parent_account_id, 
                    child_account_name, 
                    for_tapplet_name, 
                    version, 
                    tapplet_pub_key
                FROM child_accounts
                WHERE parent_account_id = ?
               "#,
                account.id
            )
            .fetch_all(&mut *conn)
            .await?;
            for child_account in child_accounts {
                scannable_accounts.push(ScannableAccount {
                    parent_account_row: account.clone(),
                    child_account_row: Some(child_account),
                });
            }
        }
    }
    Ok(scannable_accounts)
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

#[derive(sqlx::FromRow, Debug, Clone)]
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

    pub async fn get_key_manager(
        &self,
        password: &str,
    ) -> Result<TransactionKeyManagerWrapper<MemoryKeyManagerBackend>, anyhow::Error> {
        let (view_key, spend_key) = self.decrypt_keys(password)?;
        let wallet_type = Arc::new(WalletType::ProvidedKeys(ProvidedKeysWallet {
            view_key,
            birthday: Some(self.birthday as u16),
            public_spend_key: spend_key,
            private_spend_key: None,
            private_comms_key: None,
        }));
        let key_manager: TransactionKeyManagerWrapper<MemoryKeyManagerBackend> =
            TransactionKeyManagerWrapper::new(None, CryptoFactories::default(), wallet_type).await?;
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

pub async fn create_child_account_for_tapplet(
    conn: &mut SqliteConnection,
    parent_account_id: Id,
    parent_account_name: &str,
    tapplet_name: &str,
    tapplet_version: &str,
    tapplet_public_key_hex: &str,
) -> Result<Id, anyhow::Error> {
    let child_account_name = format!("{}::{}@{}", parent_account_name, tapplet_name, tapplet_version);
    let id = sqlx::query!(
        r#"
        INSERT INTO child_accounts (parent_account_id, child_account_name, for_tapplet_name, version, tapplet_pub_key)
        VALUES (?, ?, ?, ?, ?)
        RETURNING id
        "#,
        parent_account_id,
        child_account_name,
        tapplet_name,
        tapplet_version,
        tapplet_public_key_hex
    )
    .fetch_one(conn)
    .await?;
    Ok(id
        .id
        .ok_or_else(|| anyhow::anyhow!("Failed to get inserted child account ID"))?)
}

pub async fn get_child_account(
    conn: &mut SqliteConnection,
    parent_account_id: Id,
    tapplet_name: &str,
) -> Result<ChildAccountRow, anyhow::Error> {
    let row = sqlx::query_as!(
        ChildAccountRow,
        r#"
        SELECT id, 
            parent_account_id, 
            child_account_name, 
            for_tapplet_name, 
            version, 
            tapplet_pub_key
        FROM child_accounts
        WHERE parent_account_id = ? AND for_tapplet_name = ?
       "#,
        parent_account_id,
        tapplet_name
    )
    .fetch_one(&mut *conn)
    .await?;

    Ok(row)
}

pub struct ChildAccountRow {
    pub id: i64,
    pub parent_account_id: i64,
    pub child_account_name: String,
    pub for_tapplet_name: String,
    pub version: String,
    pub tapplet_pub_key: String,
}
