// Copyright 2026 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

//! Read-only access to a legacy Tari console wallet SQLite database.
//!
//! The console wallet (`tari_wallet` crate) uses Diesel and a particular schema
//! evolution. The migration only needs a small slice of that schema, so this
//! module re-implements the few read paths it requires using `rusqlite`
//! directly. We intentionally avoid pulling in the full `tari_wallet` crate as
//! a dependency; the workspace minotari-cli is built against does not include
//! it and adding it would balloon the dependency footprint substantially.
//!
//! # Cipher seed decryption
//!
//! The console wallet stores the master `CipherSeed` encrypted with a key derived
//! from the user's passphrase through this chain:
//!
//! ```text
//!     passphrase + salt --[Argon2id 46 MiB, 1 iter, 1 par, 32 byte]--> secondary_derivation_key
//!     secondary_derivation_key --[Blake2b-256, domain "com.tari.base_layer.wallet.secondary_key"]--> secondary_key
//!     XChaCha20Poly1305(secondary_key).decrypt(encrypted_main_key,
//!         AAD = b"wallet_main_key_encryption_v" + version_byte) -> main_key
//!     XChaCha20Poly1305(main_key).decrypt(encrypted_master_seed,
//!         AAD = b"wallet_setting_master_seed") -> CipherSeed bytes
//! ```
//!
//! Only Argon2 v1 (id = 1) is supported; the console wallet has had no other
//! versions to date.

use std::path::Path;

use anyhow::{Context, anyhow};
use argon2::{Algorithm, Argon2, Params, Version};
use blake2::{Blake2b, Digest};
use chacha20poly1305::{Key, KeyInit, XChaCha20Poly1305, XNonce, aead::Aead};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use tari_common_types::seeds::cipher_seed::CipherSeed;
use tari_crypto::{hash_domain, hashing::DomainSeparatedHasher};
use tari_utilities::hex::from_hex;
use zeroize::Zeroizing;

// Domain separator for the secondary key derivation. Must match the console
// wallet's `SecondaryKeyDomain` byte-for-byte.
hash_domain!(SecondaryKeyDomain, "com.tari.base_layer.wallet.secondary_key", 0);

// Argon2 parameters used by the console wallet's `Argon2Parameters::from_version(Some(1))`.
// These values are consensus-level — changing them breaks decryption of every
// existing console wallet.
const ARGON2_MEMORY_KIB: u32 = 46 * 1024;
const ARGON2_ITERATIONS: u32 = 1;
const ARGON2_PARALLELISM: u32 = 1;
const ARGON2_OUTPUT_LEN: usize = 32;
const SUPPORTED_ARGON2_VERSION: u8 = 1;

const MAIN_KEY_AAD_PREFIX: &[u8] = b"wallet_main_key_encryption_v";
const MASTER_SEED_AAD: &[u8] = b"wallet_setting_master_seed";

const XNONCE_SIZE: usize = 24;

// ── Transaction status constants ──────────────────────────────────
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

// ── Output status constants ───────────────────────────────────────
pub const OUTPUT_STATUS_UNSPENT: i64 = 0;
pub const OUTPUT_STATUS_SPENT: i64 = 1;
pub const OUTPUT_STATUS_ENCUMBERED_TO_BE_RECEIVED: i64 = 2;
pub const OUTPUT_STATUS_ENCUMBERED_TO_BE_SPENT: i64 = 3;
pub const OUTPUT_STATUS_INVALID: i64 = 4;
pub const OUTPUT_STATUS_CANCELLED_INBOUND: i64 = 5;
pub const OUTPUT_STATUS_UNSPENT_MINED_UNCONFIRMED: i64 = 6;
pub const OUTPUT_STATUS_SHORT_TERM_ENCUMBERED_TO_BE_RECEIVED: i64 = 7;
pub const OUTPUT_STATUS_SHORT_TERM_ENCUMBERED_TO_BE_SPENT: i64 = 8;
pub const OUTPUT_STATUS_SPENT_MINED_UNCONFIRMED: i64 = 9;
pub const OUTPUT_STATUS_CANCELLED_OUTBOUND: i64 = 10;

// ── Data types ────────────────────────────────────────────────────

/// Result of opening and authenticating against a legacy console wallet.
pub struct ConsoleWalletReader {
    conn: Connection,
    encrypted_master_seed: Vec<u8>,
    encrypted_main_key: Vec<u8>,
    salt: Vec<u8>,
    secondary_key_hash: Vec<u8>,
    main_key_nonce: Vec<u8>,
    master_seed_nonce: Vec<u8>,
    argon2_version: u8,
}

/// One row from the source `outputs` table, as raw column bytes.
#[derive(Debug, Clone)]
pub struct ConsoleOutputRow {
    pub commitment: Vec<u8>,
    pub spending_key: String,
    pub value: i64,
    pub output_type: i32,
    pub maturity: i64,
    pub status: i32,
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
    pub spent_in_tx_id: Option<i64>,
    pub features_json: String,
    pub covenant: Vec<u8>,
    pub mined_timestamp: Option<i64>,
    pub rangeproof: Option<Vec<u8>>,
}

/// One row from the source `completed_transactions` table.
#[derive(Debug, Clone)]
pub struct ConsoleCompletedTx {
    pub tx_id: i64,
    pub source_address: Vec<u8>,
    pub destination_address: Vec<u8>,
    pub amount: i64,
    pub fee: i64,
    pub status: i64,
    pub timestamp: i64,
    pub direction: Option<i64>,
    pub mined_height: Option<i64>,
    pub mined_in_block: Option<Vec<u8>>,
    pub payment_id: Option<Vec<u8>>,
    pub user_payment_id: Option<Vec<u8>>,
    pub message: Option<String>,
}

impl ConsoleWalletReader {
    /// Open the legacy console wallet database in read-only mode and extract
    /// the encrypted key material needed for cipher-seed decryption.
    pub fn open(path: &Path, passphrase: &str) -> anyhow::Result<Self> {
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .context("Failed to open legacy wallet DB")?;

        // Read wallet settings (encrypted key material)
        let encrypted_master_seed: Vec<u8> = conn
            .query_row(
                "SELECT value FROM wallet_settings WHERE name = 'master_seed'",
                [],
                |row| row.get(0),
            )
            .context("Missing 'master_seed' in wallet_settings")?;

        let encrypted_main_key: Vec<u8> = conn
            .query_row(
                "SELECT value FROM wallet_settings WHERE name = 'main_key'",
                [],
                |row| row.get(0),
            )
            .context("Missing 'main_key' in wallet_settings")?;

        let salt_hex: String = conn
            .query_row(
                "SELECT value FROM wallet_settings WHERE name = 'secondary_key_salt'",
                [],
                |row| row.get(0),
            )
            .context("Missing 'secondary_key_salt'")?;

        let secondary_key_hash: Vec<u8> = conn
            .query_row(
                "SELECT value FROM wallet_settings WHERE name = 'secondary_key_hash'",
                [],
                |row| row.get(0),
            )
            .context("Missing 'secondary_key_hash'")?;

        let main_key_nonce: Vec<u8> = conn
            .query_row(
                "SELECT value FROM wallet_settings WHERE name = 'main_key_nonce'",
                [],
                |row| row.get(0),
            )
            .context("Missing 'main_key_nonce'")?;

        let master_seed_nonce: Vec<u8> = conn
            .query_row(
                "SELECT value FROM wallet_settings WHERE name = 'master_seed_nonce'",
                [],
                |row| row.get(0),
            )
            .context("Missing 'master_seed_nonce'")?;

        let argon2_version: u8 = conn
            .query_row(
                "SELECT value FROM wallet_settings WHERE name = 'argon2_version'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map(|v| v as u8)
            .unwrap_or(SUPPORTED_ARGON2_VERSION);

        if argon2_version != SUPPORTED_ARGON2_VERSION {
            anyhow::bail!(
                "Unsupported Argon2 version: {} (only version {} is supported)",
                argon2_version,
                SUPPORTED_ARGON2_VERSION
            );
        }

        let salt = from_hex(&salt_hex).context("Invalid salt hex")?;

        let reader = Self {
            conn,
            encrypted_master_seed,
            encrypted_main_key,
            salt,
            secondary_key_hash,
            main_key_nonce,
            master_seed_nonce,
            argon2_version,
        };

        // Verify passphrase by deriving the secondary key and comparing
        let _passphrase_guard = Zeroizing::new(passphrase.as_bytes().to_vec());
        let derived = reader.derive_secondary_key(passphrase)?;

        if derived.as_bytes() != reader.secondary_key_hash.as_slice() {
            anyhow::bail!("Invalid passphrase: secondary key hash mismatch");
        }

        Ok(reader)
    }

    /// Decrypt and return the cipher seed from the legacy wallet.
    pub fn decrypt_cipher_seed(&self) -> anyhow::Result<CipherSeed> {
        // Step 1: Derive secondary key from passphrase
        let passphrase_key = self.derive_secondary_key_raw()?;

        // Step 2: Decrypt main key using secondary key
        let main_key = XChaCha20Poly1305::new(Key::from_slice(&passphrase_key));
        let main_key_nonce = XNonce::from_slice(&self.main_key_nonce);
        let main_key_aad = [
            MAIN_KEY_AAD_PREFIX,
            &[self.argon2_version],
        ]
        .concat();

        let main_key_bytes = main_key
            .decrypt_in_place(
                main_key_nonce,
                &main_key_aad,
                &mut self.encrypted_main_key.clone(),
            )
            .map_err(|_| anyhow!("Failed to decrypt main key (wrong passphrase?)"))?;

        // Step 3: Decrypt master seed using main key
        let seed_cipher = XChaCha20Poly1305::new(Key::from_slice(&main_key_bytes));
        let seed_nonce = XNonce::from_slice(&self.master_seed_nonce);

        let seed_bytes = seed_cipher
            .decrypt_in_place(
                seed_nonce,
                MASTER_SEED_AAD,
                &mut self.encrypted_master_seed.clone(),
            )
            .map_err(|_| anyhow!("Failed to decrypt master seed"))?;

        // Step 4: Deserialize CipherSeed
        CipherSeed::from_bytes(&seed_bytes).context("Failed to deserialize CipherSeed")
    }

    /// Fetch ALL outputs from the legacy `outputs` table.
    pub fn fetch_all_outputs(&self) -> anyhow::Result<Vec<ConsoleOutputRow>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT
                commitment, spending_key, value, output_type, maturity, status,
                hash, script, input_data, script_private_key, script_lock_height,
                sender_offset_public_key,
                metadata_signature_ephemeral_commitment,
                metadata_signature_ephemeral_pubkey,
                metadata_signature_u_a, metadata_signature_u_x, metadata_signature_u_y,
                mined_height, mined_in_block,
                received_in_tx_id, spent_in_tx_id,
                features_json, covenant, mined_timestamp, rangeproof
            FROM outputs
            "#
        )?;

        let rows = stmt.query_map([], |row| {
            Ok(ConsoleOutputRow {
                commitment: row.get(0)?,
                spending_key: row.get(1)?,
                value: row.get(2)?,
                output_type: row.get(3)?,
                maturity: row.get(4)?,
                status: row.get(5)?,
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
                spent_in_tx_id: row.get(20)?,
                features_json: row.get(21)?,
                covenant: row.get(22)?,
                mined_timestamp: row.get(23)?,
                rangeproof: row.get(24)?,
            })
        })?;

        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    /// Fetch ALL completed transactions from the legacy `completed_transactions` table.
    pub fn fetch_all_completed_transactions(&self) -> anyhow::Result<Vec<ConsoleCompletedTx>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT
                tx_id, source_address, destination_address, amount, fee,
                status, timestamp, direction, mined_height, mined_in_block,
                payment_id, user_payment_id, message
            FROM completed_transactions
            "#
        )?;

        let rows = stmt.query_map([], |row| {
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
                message: row.get(12)?,
            })
        })?;

        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    // ── Internal helpers ──────────────────────────────────────────

    fn derive_secondary_key(&self, passphrase: &str) -> anyhow::Result<blake2::digest::Output<Blake2b<argon2::digest::consts::U32>>> {
        let raw = self.derive_secondary_key_raw()?;
        // Hash through domain-separated Blake2b
        let hasher = DomainSeparatedHasher::<Blake2b<argon2::digest::consts::U32>, SecondaryKeyDomain>::new();
        Ok(hasher.chain(&raw).finalize())
    }

    fn derive_secondary_key_raw(&self) -> anyhow::Result<[u8; ARGON2_OUTPUT_LEN]> {
        let params = Params::new(
            ARGON2_MEMORY_KIB,
            ARGON2_ITERATIONS,
            ARGON2_PARALLELISM,
            Some(ARGON2_OUTPUT_LEN),
        )
        .map_err(|e| anyhow!("Invalid Argon2 params: {}", e))?;

        let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);

        let mut output = [0u8; ARGON2_OUTPUT_LEN];
        argon2
            .hash_password_into(passphrase.as_bytes(), &self.salt, &mut output)
            .map_err(|e| anyhow!("Argon2 derivation failed: {}", e))?;

        Ok(output)
    }
}
