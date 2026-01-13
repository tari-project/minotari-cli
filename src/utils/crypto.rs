use anyhow::anyhow;
use chacha20poly1305::{
    Key, KeyInit, XChaCha20Poly1305, XNonce,
    aead::{Aead, AeadCore, OsRng},
};

fn prepare_key(password: &str) -> Key {
    let mut key_bytes = [0u8; 32];
    let password_bytes = password.as_bytes();

    let len = password_bytes.len().min(32);
    key_bytes[..len].copy_from_slice(&password_bytes[..len]);

    Key::from(key_bytes)
}

/// Encrypts data using XChaCha20-Poly1305.
/// Returns (ciphertext, nonce).
pub fn encrypt_data(data: &[u8], password: &str) -> Result<(Vec<u8>, Vec<u8>), anyhow::Error> {
    let key = prepare_key(password);
    let cipher = XChaCha20Poly1305::new(&key);
    let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);

    let ciphertext = cipher
        .encrypt(&nonce, data)
        .map_err(|e| anyhow!("Encryption failed: {}", e))?;

    Ok((ciphertext, nonce.to_vec()))
}

/// Decrypts data using XChaCha20-Poly1305.
pub fn decrypt_data(encrypted_data: &[u8], nonce: &[u8], password: &str) -> Result<Vec<u8>, anyhow::Error> {
    let key = prepare_key(password);
    let cipher = XChaCha20Poly1305::new(&key);

    let nonce_bytes: &[u8; 24] = nonce.try_into().map_err(|_| anyhow!("Nonce must be 24 bytes"))?;
    let xnonce = XNonce::from(*nonce_bytes);

    let plaintext = cipher
        .decrypt(&xnonce, encrypted_data)
        .map_err(|e| anyhow!("Decryption failed: {}", e))?;

    Ok(plaintext)
}
