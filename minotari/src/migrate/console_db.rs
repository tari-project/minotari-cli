// Copyright 2026 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use std::path::Path;

use anyhow::{Context, anyhow};
use argon2::{Algorithm, Argon2, Params, Version};
use blake2::Blake2b;
use chacha20poly1305::{Key, KeyInit, XChaCha20Poly1305, XNonce, aead::Aead};
use digest::Digest;
use digest::consts::U32;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use tari_common_types::seeds::cipher_seed::CipherSeed;
use tari_crypto::{hash_domain, hashing::DomainSeparatedHasher};

// Matches tari/base_layer/wallet/src/storage/sqlite_db/wallet.rs
hash_domain!(SecondaryKeyDomain, "com.tari.base_layer.wallet.secondary_key", 0);

// Matches tari/base_layer/common_types/src/transaction.rs
pub const STATUS_COMPLETED: i64 = 0;
pub const STATUS_BROADCAST: i64 = 1;
pub const STATUS_MINED_UNCONFIRMED: i64 = 2;
pub const STATUS_IMPORTED: i64 = 3;
pub const STATUS_PENDING: i64 = 4;
pub const STATUS_COINBASE: i64 = 5;
pub const STATUS_MINED_CONFIRMED: i64 = 6;
pub const STATUS_REJECTED: i64 = 7;
pub const STATUS_ONE_SIDED_UNCONFIRMED: i64 = 8;
pub const STATUS_ONE_SIDED_CONFIRMED: i64 = 9;
pub const STATUS_QUEUED: i64 = 10;
pub const STATUS_COINBASE_UNCONFIRMED: i64 = 11;
pub const STATUS_COINBASE_CONFIRMED: i64 = 12;
pub const STATUS_COINBASE_NOT_IN_BLOCKCHAIN: i64 = 13;
pub const STATUS_MINED_CONFIRMED_LOCKED: i64 = 14;
pub const STATUS_ONE_SIDED_CONFIRMED_LOCKED: i64 = 15;
pub const STATUS_COINBASE_CONFIRMED_LOCKED: i64 = 16;

pub const OUTPUT_STATUS_UNSPENT: i64 = 0;

const ARGON2_MEMORY_KIB: u32 = 46 * 1024;
const ARGON2_ITERATIONS: u32 = 1;
const ARGON2_PARALLELISM: u32 = 1;
const DERIVED_KEY_LEN: usize = 32;
const SUPPORTED_SECONDARY_KEY_VERSION: u8 = 1;
const MAIN_KEY_AAD_PREFIX: &[u8] = b"wallet_main_key_encryption_v";
const MASTER_SEED_AAD: &[u8] = b"wallet_setting_master_seed";
const INTEGRAL_NONCE_SIZE: usize = 24;

#[derive(Debug, Clone)]
pub struct ConsoleCompletedTx {
    pub tx_id: i64,
    pub source_address: Vec<u8>,
    pub destination_address: Vec<u8>,
    pub amount: i64,
    pub fee: i64,
    pub status: i64,
    pub timestamp: chrono::NaiveDateTime,
    pub direction: Option<i64>,
    pub mined_height: Option<i64>,
    pub mined_in_block: Option<Vec<u8>>,
    pub payment_id: Option<Vec<u8>>,
    pub user_payment_id: Option<Vec<u8>>,
}

#[derive(Debug, Clone)]
pub struct ConsoleOutput {
    pub commitment: Vec<u8>,
    pub rangeproof: Option<Vec<u8>>,
    pub spending_key: String,
    pub value: i64,
    pub output_type: i64,
    pub maturity: i64,
    pub hash: Vec<u8>,
    pub script: Vec<u8>,
    pub input_data: Vec<u8>,
    pub script_private_key: String,
    pub script_lock_height: i64,
    pub sender_offset_public_key: Vec<u8>,
    pub metadata_signature_ephemeral_commitment: Vec<u8>,
    pub metadata_signature_ephemeral_pubkey: Vec<u8>,
    pub metadata_signature_u_a: Vec<u8>,
    pub metadata_signature_u_x: Vec<u8>,
    pub metadata_signature_u_y: Vec<u8>,
    pub mined_height: Option<i64>,
    pub mined_in_block: Option<Vec<u8>>,
    pub received_in_tx_id: Option<i64>,
    pub features_json: String,
    pub covenant: Vec<u8>,
    pub mined_timestamp: Option<chrono::NaiveDateTime>,
    pub encrypted_data: Vec<u8>,
    pub minimum_value_promise: i64,
    pub payment_id: Option<Vec<u8>>,
}

#[derive(Debug, Clone)]
pub struct ConsoleSyncTip {
    pub height: u64,
    pub block_hash: Vec<u8>,
}

pub struct ConsoleDb {
    conn: Connection,
}

impl ConsoleDb {
    pub fn open(path: &Path, passphrase: &str) -> Result<(Self, CipherSeed), anyhow::Error> {
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .with_context(|| format!("Failed to open source console wallet DB at {}", path.display()))?;

        let db = Self { conn };
        let cipher_seed = db.derive_cipher_seed(passphrase)?;
        Ok((db, cipher_seed))
    }

    pub fn read_completed_transactions(&self) -> Result<Vec<ConsoleCompletedTx>, anyhow::Error> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT
                tx_id,
                source_address,
                destination_address,
                amount,
                fee,
                status,
                timestamp,
                direction,
                mined_height,
                mined_in_block,
                payment_id,
                user_payment_id
            FROM completed_transactions
            WHERE (cancelled IS NULL OR cancelled = 0)
              AND status NOT IN (?1, ?2)
            ORDER BY tx_id ASC
            "#,
        )?;

        let rows = stmt
            .query_map(params![STATUS_PENDING, STATUS_REJECTED], |row| {
                Ok(ConsoleCompletedTx {
                    tx_id: row.get(0)?,
                    source_address: row.get(1)?,
                    destination_address: row.get(2)?,
                    amount: row.get(3)?,
                    fee: row.get(4)?,
                    status: row.get(5)?,
                    timestamp: row.get(6)?,
                    direction: row.get(7)?,
                    mined_height: row.get(8)?,
                    mined_in_block: row.get(9)?,
                    payment_id: row.get(10)?,
                    user_payment_id: row.get(11)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(rows)
    }

    pub fn read_unspent_outputs(&self) -> Result<Vec<ConsoleOutput>, anyhow::Error> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT
                commitment,
                rangeproof,
                spending_key,
                value,
                output_type,
                maturity,
                hash,
                script,
                input_data,
                script_private_key,
                script_lock_height,
                sender_offset_public_key,
                metadata_signature_ephemeral_commitment,
                metadata_signature_ephemeral_pubkey,
                metadata_signature_u_a,
                metadata_signature_u_x,
                metadata_signature_u_y,
                mined_height,
                mined_in_block,
                received_in_tx_id,
                features_json,
                covenant,
                mined_timestamp,
                encrypted_data,
                minimum_value_promise,
                payment_id
            FROM outputs
            WHERE status = ?1
            ORDER BY id ASC
            "#,
        )?;

        let rows = stmt
            .query_map(params![OUTPUT_STATUS_UNSPENT], |row| {
                Ok(ConsoleOutput {
                    commitment: row.get(0)?,
                    rangeproof: row.get(1)?,
                    spending_key: row.get(2)?,
                    value: row.get(3)?,
                    output_type: row.get(4)?,
                    maturity: row.get(5)?,
                    hash: row.get(6)?,
                    script: row.get(7)?,
                    input_data: row.get(8)?,
                    script_private_key: row.get(9)?,
                    script_lock_height: row.get(10)?,
                    sender_offset_public_key: row.get(11)?,
                    metadata_signature_ephemeral_commitment: row.get(12)?,
                    metadata_signature_ephemeral_pubkey: row.get(13)?,
                    metadata_signature_u_a: row.get(14)?,
                    metadata_signature_u_x: row.get(15)?,
                    metadata_signature_u_y: row.get(16)?,
                    mined_height: row.get(17)?,
                    mined_in_block: row.get(18)?,
                    received_in_tx_id: row.get(19)?,
                    features_json: row.get(20)?,
                    covenant: row.get(21)?,
                    mined_timestamp: row.get(22)?,
                    encrypted_data: row.get(23)?,
                    minimum_value_promise: row.get(24)?,
                    payment_id: row.get(25)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(rows)
    }

    pub fn read_sync_tip(&self) -> Result<Option<ConsoleSyncTip>, anyhow::Error> {
        if self.table_exists("scanned_blocks")? {
            let tip = self
                .conn
                .query_row(
                    "SELECT height, header_hash FROM scanned_blocks ORDER BY height DESC LIMIT 1",
                    [],
                    |row| {
                        let height_i64: i64 = row.get(0)?;
                        let height = height_i64.try_into().map_err(|_| {
                            rusqlite::Error::FromSqlConversionFailure(
                                0,
                                rusqlite::types::Type::Integer,
                                "Invalid negative height".into(),
                            )
                        })?;
                        let block_hash: Vec<u8> = row.get(1)?;
                        Ok(ConsoleSyncTip { height, block_hash })
                    },
                )
                .optional()?;

            if tip.is_some() {
                return Ok(tip);
            }
        }

        if self.table_exists("sync_tip")? {
            for query in [
                "SELECT height, hash FROM sync_tip ORDER BY height DESC LIMIT 1",
                "SELECT height, header_hash FROM sync_tip ORDER BY height DESC LIMIT 1",
            ] {
                if let Ok(mut stmt) = self.conn.prepare(query) {
                    let tip = stmt
                        .query_row([], |row| {
                            let height_i64: i64 = row.get(0)?;
                            let height = height_i64.try_into().map_err(|_| {
                                rusqlite::Error::FromSqlConversionFailure(
                                    0,
                                    rusqlite::types::Type::Integer,
                                    "Invalid negative height".into(),
                                )
                            })?;
                            let block_hash: Vec<u8> = row.get(1)?;
                            Ok(ConsoleSyncTip { height, block_hash })
                        })
                        .optional()?;

                    if tip.is_some() {
                        return Ok(tip);
                    }
                }
            }
        }

        Ok(None)
    }

    pub fn fallback_max_mined_height(&self) -> Result<Option<u64>, anyhow::Error> {
        let max_height = self
            .conn
            .query_row(
                "SELECT MAX(mined_height) FROM outputs WHERE status = ?1",
                params![OUTPUT_STATUS_UNSPENT],
                |row| row.get::<_, Option<i64>>(0),
            )
            .optional()?
            .flatten()
            .and_then(|height| height.try_into().ok());

        Ok(max_height)
    }

    fn table_exists(&self, table_name: &str) -> Result<bool, anyhow::Error> {
        let exists = self
            .conn
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1 LIMIT 1",
                params![table_name],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
            .is_some();
        Ok(exists)
    }

    fn read_wallet_setting(&self, key: &str) -> Result<String, anyhow::Error> {
        self.conn
            .query_row(
                "SELECT value FROM wallet_settings WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| anyhow!("Console wallet setting '{key}' is missing"))
    }

    fn derive_cipher_seed(&self, passphrase: &str) -> Result<CipherSeed, anyhow::Error> {
        let secondary_key_version = self.read_wallet_setting("SecondaryKeyVersion")?;
        let secondary_key_salt = self.read_wallet_setting("SecondaryKeySalt")?;
        let secondary_key_hash_hex = self.read_wallet_setting("SecondaryKeyHash")?;
        let encrypted_main_key_hex = self.read_wallet_setting("EncryptedMainKey")?;
        let encrypted_master_seed_hex = self.read_wallet_setting("MasterSeed")?;

        let version: u8 = secondary_key_version
            .parse()
            .with_context(|| format!("Invalid SecondaryKeyVersion '{}'", secondary_key_version))?;
        if version != SUPPORTED_SECONDARY_KEY_VERSION {
            return Err(anyhow!(
                "Unsupported console wallet encryption version {}. Open the wallet once with the latest console wallet binary, then retry.",
                version
            ));
        }

        let mut secondary_derivation_key = [0u8; DERIVED_KEY_LEN];
        let params = Params::new(
            ARGON2_MEMORY_KIB,
            ARGON2_ITERATIONS,
            ARGON2_PARALLELISM,
            Some(DERIVED_KEY_LEN),
        )?;
        let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
        argon2
            .hash_password_into(
                passphrase.as_bytes(),
                secondary_key_salt.as_bytes(),
                &mut secondary_derivation_key,
            )
            .context("Failed to derive console wallet secondary key")?;

        let derived_secondary_key = DomainSeparatedHasher::<Blake2b<U32>, SecondaryKeyDomain>::new()
            .chain_update(secondary_derivation_key)
            .finalize();
        let derived_secondary_key = derived_secondary_key.as_ref();
        let expected_secondary_key_hash =
            hex::decode(&secondary_key_hash_hex).context("SecondaryKeyHash is not valid hex")?;

        if derived_secondary_key != expected_secondary_key_hash.as_slice() {
            return Err(anyhow!("Console wallet password is incorrect"));
        }

        let secondary_key = Key::try_from(derived_secondary_key).expect("key must be 32 bytes");
        let secondary_cipher = XChaCha20Poly1305::new(&secondary_key);

        let mut main_key_aad = MAIN_KEY_AAD_PREFIX.to_vec();
        main_key_aad.push(version);

        let encrypted_main_key = hex::decode(&encrypted_main_key_hex).context("EncryptedMainKey is not valid hex")?;
        let main_key = decrypt_integral_nonce(&secondary_cipher, &main_key_aad, &encrypted_main_key)
            .context("Failed to decrypt EncryptedMainKey")?;

        let main_key = Key::try_from(main_key.as_ref()).map_err(|_| anyhow!("Decrypted main key has invalid length (expected 32 bytes)"))?;
        let main_cipher = XChaCha20Poly1305::new(&main_key);

        let encrypted_master_seed = hex::decode(&encrypted_master_seed_hex).context("MasterSeed is not valid hex")?;
        let seed_bytes = decrypt_integral_nonce(&main_cipher, MASTER_SEED_AAD, &encrypted_master_seed)
            .context("Failed to decrypt MasterSeed")?;

        CipherSeed::from_enciphered_bytes(&seed_bytes, None)
            .map_err(|e| anyhow!("Failed to decode CipherSeed from source wallet: {e}"))
    }
}

fn decrypt_integral_nonce(cipher: &XChaCha20Poly1305, aad: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>, anyhow::Error> {
    if ciphertext.len() <= INTEGRAL_NONCE_SIZE {
        return Err(anyhow!(
            "Encrypted source value is too short to contain an XChaCha20 nonce"
        ));
    }

    let (nonce_bytes, encrypted_bytes) = ciphertext.split_at(INTEGRAL_NONCE_SIZE);
    let nonce = XNonce::try_from(nonce_bytes).expect("nonce must be 24 bytes");
    cipher
        .decrypt(
            &nonce,
            chacha20poly1305::aead::Payload {
                msg: encrypted_bytes,
                aad,
            },
        )
        .map_err(|e| anyhow!("XChaCha20Poly1305 decrypt failed: {e}"))
}
