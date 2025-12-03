use blake2::Blake2s256;
use blake2::Digest;
use chacha20poly1305::AeadCore;
use chacha20poly1305::KeyInit;
use chacha20poly1305::aead::Aead;
use chacha20poly1305::{Key, XChaCha20Poly1305, aead::OsRng};
pub fn encrypt_with_password(
    password: &str,
    view_key_bytes: &Vec<u8>,
    spend_key_bytes: Vec<u8>,
) -> Result<(Vec<u8>, Vec<u8>, Vec<u8>), anyhow::Error> {
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
    Ok((nonce.to_vec(), encrypted_view_key, encrypted_spend_key))
}

pub fn hash_view_key(view_key: &[u8]) -> Vec<u8> {
    let mut hasher = Blake2s256::new();
    hasher.update(b"view_key_hash");
    hasher.update(view_key);
    hasher.finalize().to_vec()
}
