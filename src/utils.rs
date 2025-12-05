use blake2::{Blake2s256, Digest};
use chacha20poly1305::{
    Key, KeyInit, XChaCha20Poly1305,
    aead::{Aead, AeadCore, OsRng},
};

use crate::{db, init_db};

/// Initialize a new wallet account with the given view key, spend key, and password.
///
/// Includes database initialization logic
pub async fn init_with_view_key(
    view_private_key: &str,
    spend_public_key: &str,
    password: &str,
    database_file: &str,
    birthday: u16,
    friendly_name: Option<&str>,
) -> Result<(), anyhow::Error> {
    let view_key_bytes = hex::decode(view_private_key)?;
    let spend_key_bytes = hex::decode(spend_public_key)?;

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

    let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
    let encrypted_view_key = cipher.encrypt(&nonce, view_key_bytes.as_ref())?;
    let encrypted_spend_key = cipher.encrypt(&nonce, spend_key_bytes.as_ref())?;

    // create a hash of the viewkey to determine duplicate wallets
    let view_key_hash = hash_view_key(&view_key_bytes);
    let pool = init_db(database_file).await?;
    let mut conn = pool.acquire().await?;
    let friendly_name = friendly_name.unwrap_or("default");
    db::create_account(
        &mut conn,
        friendly_name,
        &encrypted_view_key,
        &encrypted_spend_key,
        &nonce,
        &view_key_hash,
        birthday as i64,
    )
    .await?;

    Ok(())
}

fn hash_view_key(view_key: &[u8]) -> Vec<u8> {
    let mut hasher = Blake2s256::new();
    hasher.update(b"view_key_hash");
    hasher.update(view_key);
    hasher.finalize().to_vec()
}
