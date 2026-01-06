//! Utility functions for wallet initialization and key management.
//!
//! This module provides helper functions for initializing new wallet accounts
//! with encrypted key storage. It handles password padding, key encryption,
//! and database initialization.

use blake2::{Blake2s256, Digest};
use chacha20poly1305::{
    Key, KeyInit, XChaCha20Poly1305,
    aead::{Aead, AeadCore, OsRng},
};
use zeroize::Zeroizing;
use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};

use crate::{db, init_db};

/// Initializes a new wallet account with view and spend keys.
///
/// This function creates a new wallet account by:
/// 1. Decoding the provided hex-encoded keys
/// 2. Padding the password to 32 bytes if necessary
/// 3. Encrypting both keys using XChaCha20-Poly1305
/// 4. Generating a hash of the view key to prevent duplicates
/// 5. Storing the encrypted keys in the database
///
/// # Parameters
///
/// * `view_private_key` - Hex-encoded view private key
/// * `spend_public_key` - Hex-encoded spend public key
/// * `password` - Password for encrypting the keys (will be padded to 32 bytes)
/// * `database_file` - Path to the SQLite database file
/// * `birthday` - Block height to start scanning from (0 to scan entire chain)
/// * `friendly_name` - Optional account name (defaults to "default")
///
/// # Returns
///
/// Returns `Ok(())` on success or an error if:
/// - Keys cannot be decoded from hex
/// - Encryption fails
/// - Database operations fail
/// - Account with same view key already exists
///
/// # Security
///
/// - Keys are encrypted using XChaCha20-Poly1305 with a random nonce
/// - Passwords shorter than 32 bytes are padded with zeros
/// - Passwords longer than 32 bytes are truncated
/// - View key hash prevents duplicate account creation
///
/// # Example
///
/// ```no_run
/// use minotari::utils::init_with_view_key;
///
/// # async fn example() -> Result<(), anyhow::Error> {
/// init_with_view_key(
///     "a0b1c2d3e4f5...",  // view_private_key (hex)
///     "b1c2d3e4f5a0...",  // spend_public_key (hex)
///     "my_secure_password",
///     "wallet.db",
///     0,                   // birthday height
///     Some("my_wallet"),   // account name
/// ).await?;
/// # Ok(())
/// # }
/// ```
pub fn init_with_view_key(
    view_private_key: &str,
    spend_public_key: &str,
    password: &Zeroizing<String>,
    database_file: &str,
    birthday: u16,
    friendly_name: Option<&str>,
) -> Result<(), anyhow::Error> {
    let view_key_bytes = hex::decode(view_private_key)?;
    let spend_key_bytes = hex::decode(spend_public_key)?;

    let password = if password.len() < 32 {
        Zeroizing::new(format!("{:0<32}", password.as_str()))
    } else {
        if password.len() > 32 {
            return Err(anyhow::anyhow!("Password must be at most 32 bytes"));
        }
        password.clone()
    };
    let key_bytes: [u8; 32] = password
        .as_bytes()
        .try_into()
        .map_err(|_| anyhow::anyhow!("Password must be 32 bytes"))?;
    let key = Key::from(key_bytes);
    let cipher = XChaCha20Poly1305::new(&key);

    let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
    let encrypted_view_key = cipher.encrypt(&nonce, view_key_bytes.as_ref())?;
    let encrypted_spend_key = cipher.encrypt(&nonce, spend_key_bytes.as_ref())?;

    // create a hash of the viewkey to determine duplicate wallets
    let view_key_hash = hash_view_key(&view_key_bytes);
    let pool = init_db(database_file)?;
    let conn = pool.get()?;
    let friendly_name = friendly_name.unwrap_or("default");
    db::create_account(
        &conn,
        friendly_name,
        &encrypted_view_key,
        &encrypted_spend_key,
        &nonce,
        &view_key_hash,
        birthday as i64,
    )?;

    Ok(())
}

/// Computes a Blake2s-256 hash of the view key for duplicate detection.
///
/// This hash is stored in the database to prevent creating multiple accounts
/// with the same view key, which would cause scanning conflicts.
///
/// # Parameters
///
/// * `view_key` - Raw bytes of the view private key
///
/// # Returns
///
/// 32-byte Blake2s-256 hash of the view key with domain separation
fn hash_view_key(view_key: &[u8]) -> Vec<u8> {
    let mut hasher = Blake2s256::new();
    hasher.update(b"view_key_hash");
    hasher.update(view_key);
    hasher.finalize().to_vec()
}

pub trait AsNaive {
    fn as_naive(&self) -> NaiveDateTime;
}

impl AsNaive for NaiveDateTime {
    fn as_naive(&self) -> NaiveDateTime {
        *self
    }
}

impl<T: TimeZone> AsNaive for DateTime<T> {
    fn as_naive(&self) -> NaiveDateTime {
        self.naive_utc()
    }
}

pub fn format_timestamp(date: impl AsNaive) -> String {
    date.as_naive().format("%Y-%m-%d %H:%M:%S").to_string()
}

pub fn current_db_timestamp() -> String {
    format_timestamp(Utc::now())
}
