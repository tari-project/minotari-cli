// Copyright 2026 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

#![allow(clippy::indexing_slicing)]
#![allow(clippy::too_many_lines)]

use std::path::{Path, PathBuf};

use argon2::{Algorithm, Argon2, Params, Version};
use blake2::Blake2b;
use borsh::BorshSerialize;
use chacha20poly1305::{
    Key, KeyInit, XChaCha20Poly1305, XNonce,
    aead::{Aead, Payload},
};
use digest::Digest;
use digest::consts::U32;
use rand::RngCore;
use rusqlite::{Connection, params};
use tari_common_types::seeds::cipher_seed::CipherSeed;
use tari_crypto::{hash_domain, hashing::DomainSeparatedHasher};
use tari_script::{inputs, script};
use tari_transaction_components::{
    MicroMinotari,
    key_manager::{
        KeyManager, TransactionKeyManagerInterface, wallet_types::SeedWordsWallet, wallet_types::WalletType,
    },
    transaction_components::{MemoField, OutputFeatures, WalletOutput, WalletOutputBuilder, covenants::Covenant},
};
use tari_utilities::ByteArray;
use tempfile::TempDir;

use crate::db::{
    get_account_by_name, get_displayed_transactions_by_account, get_latest_scanned_tip_block_by_account,
    get_total_unspent_balance, init_db,
};

use super::{
    console_db::{ConsoleDb, STATUS_MINED_CONFIRMED},
    migrator::{MigrationOptions, MigrationReport, run_migration},
};

hash_domain!(SecondaryKeyDomain, "com.tari.base_layer.wallet.secondary_key", 0);

const ARGON2_MEMORY_KIB: u32 = 46 * 1024;
const ARGON2_ITERATIONS: u32 = 1;
const ARGON2_PARALLELISM: u32 = 1;
const DERIVED_KEY_LEN: usize = 32;
const SECONDARY_KEY_VERSION: u8 = 1;
const MAIN_KEY_AAD_PREFIX: &[u8] = b"wallet_main_key_encryption_v";
const MASTER_SEED_AAD: &[u8] = b"wallet_setting_master_seed";

struct TestContext {
    _temp_dir: TempDir,
    source_db_path: PathBuf,
    dest_db_path: PathBuf,
    source_passphrase: String,
    source_seed: CipherSeed,
}

struct OutputSpec {
    value: u64,
    received_in_tx_id: Option<i64>,
    mined_height: i64,
    block_seed: u8,
    legacy_spending_key: bool,
    legacy_script_key: bool,
}

impl OutputSpec {
    fn new(value: u64, received_in_tx_id: Option<i64>, mined_height: i64, block_seed: u8) -> Self {
        Self {
            value,
            received_in_tx_id,
            mined_height,
            block_seed,
            legacy_spending_key: false,
            legacy_script_key: false,
        }
    }
}

#[test]
fn migration_preserves_displayed_transaction_ids() {
    let ctx = create_test_context("preserve_ids");
    let source_key_manager = source_key_manager(&ctx.source_seed);

    insert_completed_transaction(&ctx.source_db_path, 1001, 500, 5, 0, STATUS_MINED_CONFIRMED, Some(101));
    insert_completed_transaction(&ctx.source_db_path, 1002, 700, 7, 1, STATUS_MINED_CONFIRMED, Some(102));
    insert_completed_transaction(&ctx.source_db_path, 1003, 900, 9, 0, STATUS_MINED_CONFIRMED, Some(103));
    insert_output(
        &ctx.source_db_path,
        &source_key_manager,
        OutputSpec::new(500, Some(1001), 101, 0x11),
    );
    insert_output(
        &ctx.source_db_path,
        &source_key_manager,
        OutputSpec::new(900, Some(1003), 103, 0x13),
    );
    insert_scanned_block(&ctx.source_db_path, 103, fixed_hash_bytes(0x13));

    let (report, pool) = run_live_migration(&ctx, false);
    assert_eq!(report.transactions_imported, 3);

    let conn = pool.get().expect("destination connection");
    let account = get_account_by_name(&conn, "imported")
        .expect("load account")
        .expect("account exists");
    let displayed = get_displayed_transactions_by_account(&conn, account.id).expect("displayed tx query");
    let mut ids = displayed.into_iter().map(|tx| tx.id.to_string()).collect::<Vec<_>>();
    ids.sort();

    assert_eq!(ids, vec!["1001".to_string(), "1002".to_string(), "1003".to_string()]);
}

#[test]
fn migration_handles_duplicate_received_in_tx_id() {
    let ctx = create_test_context("duplicate_tx_id");
    let source_key_manager = source_key_manager(&ctx.source_seed);

    insert_completed_transaction(&ctx.source_db_path, 2000, 300, 3, 0, STATUS_MINED_CONFIRMED, Some(200));
    insert_output(
        &ctx.source_db_path,
        &source_key_manager,
        OutputSpec::new(100, Some(2000), 200, 0x21),
    );
    insert_output(
        &ctx.source_db_path,
        &source_key_manager,
        OutputSpec::new(200, Some(2000), 200, 0x22),
    );
    insert_scanned_block(&ctx.source_db_path, 200, fixed_hash_bytes(0x22));

    let (_report, pool) = run_live_migration(&ctx, false);
    let conn = pool.get().expect("destination connection");

    let mut stmt = conn
        .prepare("SELECT tx_id, status FROM outputs ORDER BY id ASC")
        .expect("prepare outputs query");
    let rows = stmt
        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
        .expect("query outputs")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect outputs");

    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].1, "UNSPENT");
    assert_eq!(rows[1].1, "UNSPENT");
    assert_eq!(rows.iter().filter(|(tx_id, _)| *tx_id == 2000).count(), 1);
    assert_ne!(rows[0].0, rows[1].0);
}

#[test]
fn migration_balance_matches_source() {
    let ctx = create_test_context("balance_match");
    let source_key_manager = source_key_manager(&ctx.source_seed);

    insert_completed_transaction(&ctx.source_db_path, 3001, 100, 1, 0, STATUS_MINED_CONFIRMED, Some(301));
    insert_completed_transaction(&ctx.source_db_path, 3002, 200, 2, 0, STATUS_MINED_CONFIRMED, Some(302));
    insert_completed_transaction(&ctx.source_db_path, 3003, 300, 3, 0, STATUS_MINED_CONFIRMED, Some(303));
    insert_output(
        &ctx.source_db_path,
        &source_key_manager,
        OutputSpec::new(100, Some(3001), 301, 0x31),
    );
    insert_output(
        &ctx.source_db_path,
        &source_key_manager,
        OutputSpec::new(200, Some(3002), 302, 0x32),
    );
    insert_output(
        &ctx.source_db_path,
        &source_key_manager,
        OutputSpec::new(300, Some(3003), 303, 0x33),
    );

    let (_report, pool) = run_live_migration(&ctx, false);
    let conn = pool.get().expect("destination connection");
    let account = get_account_by_name(&conn, "imported")
        .expect("load account")
        .expect("account exists");
    let balance = get_total_unspent_balance(&conn, account.id).expect("balance query");

    assert_eq!(balance, 600);
}

#[test]
fn dry_run_writes_nothing() {
    let ctx = create_test_context("dry_run");
    let source_key_manager = source_key_manager(&ctx.source_seed);

    insert_completed_transaction(&ctx.source_db_path, 4001, 120, 1, 0, STATUS_MINED_CONFIRMED, Some(401));
    insert_completed_transaction(&ctx.source_db_path, 4002, 220, 2, 0, STATUS_MINED_CONFIRMED, Some(402));
    insert_output(
        &ctx.source_db_path,
        &source_key_manager,
        OutputSpec::new(120, Some(4001), 401, 0x41),
    );
    insert_output(
        &ctx.source_db_path,
        &source_key_manager,
        OutputSpec::new(220, Some(4002), 402, 0x42),
    );

    let (report, pool) = run_dry_run(&ctx, false);
    assert!(report.dry_run);

    let conn = pool.get().expect("destination connection");
    for table in [
        "accounts",
        "displayed_transactions",
        "outputs",
        "balance_changes",
        "scanned_tip_blocks",
    ] {
        let count: i64 = conn
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| row.get(0))
            .expect("count rows");
        assert_eq!(count, 0, "table {table} should remain empty");
    }
}

#[test]
fn legacy_key_outputs_block_dry_run_and_abort_live_by_default() {
    let ctx = create_test_context("legacy_blocker");
    let source_key_manager = source_key_manager(&ctx.source_seed);

    insert_completed_transaction(&ctx.source_db_path, 5001, 123, 1, 0, STATUS_MINED_CONFIRMED, Some(501));
    insert_output(
        &ctx.source_db_path,
        &source_key_manager,
        OutputSpec {
            legacy_spending_key: true,
            ..OutputSpec::new(123, Some(5001), 501, 0x51)
        },
    );

    let (dry_run_report, pool) = run_dry_run(&ctx, false);
    assert_eq!(dry_run_report.outputs_blocked_legacy_key, 1);
    assert_eq!(dry_run_report.outputs_imported, 0);
    assert_eq!(dry_run_report.blocked_output_hashes.len(), 1);

    let conn = pool.get().expect("destination connection");
    let account_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM accounts", [], |row| row.get(0))
        .expect("count accounts");
    assert_eq!(account_count, 0);

    let error = run_migration_from_context(&ctx, false, false).expect_err("live migration should abort");
    let error_text = error.to_string();
    assert!(error_text.contains("Legacy key format detected for output"));
    assert!(error_text.contains("upgrade your wallet's key format"));

    let conn = init_db(ctx.dest_db_path.clone())
        .expect("init db")
        .get()
        .expect("destination connection");
    let account_count_after: i64 = conn
        .query_row("SELECT COUNT(*) FROM accounts", [], |row| row.get(0))
        .expect("count accounts");
    assert_eq!(account_count_after, 0);
}

#[test]
fn scan_tip_prefers_explicit_sync_metadata_and_falls_back_to_max_output_height() {
    let explicit_ctx = create_test_context("explicit_scan_tip");
    let explicit_key_manager = source_key_manager(&explicit_ctx.source_seed);
    insert_completed_transaction(
        &explicit_ctx.source_db_path,
        6001,
        100,
        1,
        0,
        STATUS_MINED_CONFIRMED,
        Some(601),
    );
    insert_output(
        &explicit_ctx.source_db_path,
        &explicit_key_manager,
        OutputSpec::new(100, Some(6001), 601, 0x61),
    );
    insert_output(
        &explicit_ctx.source_db_path,
        &explicit_key_manager,
        OutputSpec::new(200, Some(6001), 602, 0x62),
    );
    insert_sync_tip(&explicit_ctx.source_db_path, 9999, fixed_hash_bytes(0x6f));

    let (_report, explicit_pool) = run_live_migration(&explicit_ctx, false);
    let explicit_conn = explicit_pool.get().expect("destination connection");
    let explicit_account = get_account_by_name(&explicit_conn, "imported")
        .expect("load account")
        .expect("account exists");
    let explicit_tip = get_latest_scanned_tip_block_by_account(&explicit_conn, explicit_account.id)
        .expect("scan tip query")
        .expect("scan tip exists");
    assert_eq!(explicit_tip.height, 9999);
    assert_eq!(explicit_tip.hash, fixed_hash_bytes(0x6f));

    let fallback_ctx = create_test_context("fallback_scan_tip");
    let fallback_key_manager = source_key_manager(&fallback_ctx.source_seed);
    insert_completed_transaction(
        &fallback_ctx.source_db_path,
        7001,
        100,
        1,
        0,
        STATUS_MINED_CONFIRMED,
        Some(701),
    );
    insert_output(
        &fallback_ctx.source_db_path,
        &fallback_key_manager,
        OutputSpec::new(100, Some(7001), 700, 0x71),
    );
    insert_output(
        &fallback_ctx.source_db_path,
        &fallback_key_manager,
        OutputSpec::new(200, Some(7001), 800, 0x72),
    );

    let (_report, fallback_pool) = run_live_migration(&fallback_ctx, false);
    let fallback_conn = fallback_pool.get().expect("destination connection");
    let fallback_account = get_account_by_name(&fallback_conn, "imported")
        .expect("load account")
        .expect("account exists");
    let fallback_tip = get_latest_scanned_tip_block_by_account(&fallback_conn, fallback_account.id)
        .expect("scan tip query")
        .expect("scan tip exists");
    assert_eq!(fallback_tip.height, 800);
    assert_eq!(fallback_tip.hash, fixed_hash_bytes(0x72));
}

#[test]
fn allow_partial_import_skips_legacy_outputs_and_reports_them() {
    let ctx = create_test_context("allow_partial");
    let source_key_manager = source_key_manager(&ctx.source_seed);

    insert_completed_transaction(&ctx.source_db_path, 8001, 100, 1, 0, STATUS_MINED_CONFIRMED, Some(801));
    insert_output(
        &ctx.source_db_path,
        &source_key_manager,
        OutputSpec::new(100, Some(8001), 801, 0x81),
    );
    insert_output(
        &ctx.source_db_path,
        &source_key_manager,
        OutputSpec {
            legacy_script_key: true,
            ..OutputSpec::new(250, Some(8001), 802, 0x82)
        },
    );

    let (report, pool) = run_live_migration(&ctx, true);
    assert!(report.partial_import);
    assert_eq!(report.outputs_imported, 1);
    assert_eq!(report.outputs_skipped_legacy_key, 1);
    assert_eq!(report.source_balance_utari, 350);
    assert_eq!(report.imported_balance_utari, 100);
    assert!(!report.balance_match);

    let conn = pool.get().expect("destination connection");
    let output_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM outputs", [], |row| row.get(0))
        .expect("count outputs");
    assert_eq!(output_count, 1);
}

fn create_test_context(name: &str) -> TestContext {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let source_db_path = temp_dir.path().join(format!("{name}_source.sqlite3"));
    let dest_db_path = temp_dir.path().join(format!("{name}_dest.sqlite3"));
    let source_passphrase = "source-wallet-passphrase".to_string();
    let source_seed = CipherSeed::random();

    let conn = Connection::open(&source_db_path).expect("open source db");
    create_source_schema(&conn);
    write_wallet_settings(&conn, &source_seed, &source_passphrase);

    TestContext {
        _temp_dir: temp_dir,
        source_db_path,
        dest_db_path,
        source_passphrase,
        source_seed,
    }
}

fn create_source_schema(conn: &Connection) {
    conn.execute_batch(
        r#"
        CREATE TABLE wallet_settings (
            key TEXT PRIMARY KEY NOT NULL,
            value TEXT NOT NULL
        );

        CREATE TABLE outputs (
            id INTEGER PRIMARY KEY NOT NULL,
            commitment BLOB NOT NULL,
            rangeproof BLOB NULL,
            spending_key TEXT NOT NULL,
            value BIGINT NOT NULL,
            output_type INTEGER NOT NULL,
            maturity BIGINT NOT NULL,
            status INTEGER NOT NULL,
            hash BLOB NOT NULL,
            script BLOB NOT NULL,
            input_data BLOB NOT NULL,
            script_private_key TEXT NOT NULL,
            script_lock_height BIGINT NOT NULL DEFAULT 0,
            sender_offset_public_key BLOB NOT NULL,
            metadata_signature_ephemeral_commitment BLOB NOT NULL,
            metadata_signature_ephemeral_pubkey BLOB NOT NULL,
            metadata_signature_u_a BLOB NOT NULL,
            metadata_signature_u_x BLOB NOT NULL,
            metadata_signature_u_y BLOB NOT NULL,
            mined_height BIGINT NULL,
            mined_in_block BLOB NULL,
            received_in_tx_id BIGINT NULL,
            features_json TEXT NOT NULL DEFAULT '{}',
            covenant BLOB NOT NULL,
            mined_timestamp DATETIME NULL,
            encrypted_data BLOB NOT NULL,
            minimum_value_promise BIGINT NOT NULL,
            payment_id BLOB NULL
        );

        CREATE TABLE completed_transactions (
            tx_id BIGINT PRIMARY KEY NOT NULL,
            source_address BLOB NOT NULL,
            destination_address BLOB NOT NULL,
            amount BIGINT NOT NULL,
            fee BIGINT NOT NULL,
            transaction_protocol BLOB NOT NULL,
            status INTEGER NOT NULL,
            timestamp DATETIME NOT NULL,
            cancelled INTEGER NULL,
            direction INTEGER NULL,
            mined_height BIGINT NULL,
            mined_in_block BLOB NULL,
            payment_id BLOB NULL,
            user_payment_id BLOB NULL
        );

        CREATE TABLE scanned_blocks (
            header_hash BLOB PRIMARY KEY NOT NULL,
            height BIGINT NOT NULL,
            num_outputs BIGINT NULL,
            amount BIGINT NULL,
            timestamp DATETIME NOT NULL
        );

        CREATE TABLE sync_tip (
            id INTEGER PRIMARY KEY NOT NULL,
            height BIGINT NOT NULL,
            header_hash BLOB NOT NULL
        );
        "#,
    )
    .expect("create source schema");
}

fn write_wallet_settings(conn: &Connection, seed: &CipherSeed, passphrase: &str) {
    let mut secondary_derivation_key = [0u8; DERIVED_KEY_LEN];
    let salt = random_hex(16);
    let params = Params::new(
        ARGON2_MEMORY_KIB,
        ARGON2_ITERATIONS,
        ARGON2_PARALLELISM,
        Some(DERIVED_KEY_LEN),
    )
    .expect("argon2 params");
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    argon2
        .hash_password_into(passphrase.as_bytes(), salt.as_bytes(), &mut secondary_derivation_key)
        .expect("derive secondary key");

    let secondary_key = DomainSeparatedHasher::<Blake2b<U32>, SecondaryKeyDomain>::new()
        .chain_update(secondary_derivation_key)
        .finalize();
    let secondary_cipher =
        XChaCha20Poly1305::new(&Key::try_from(secondary_key.as_ref()).expect("key must be 32 bytes"));

    let mut main_key = [0u8; DERIVED_KEY_LEN];
    rand::thread_rng().fill_bytes(&mut main_key);

    let mut aad = MAIN_KEY_AAD_PREFIX.to_vec();
    aad.push(SECONDARY_KEY_VERSION);
    let encrypted_main_key = encrypt_integral_nonce(&secondary_cipher, &aad, &main_key);

    let main_cipher = XChaCha20Poly1305::new(&Key::try_from(main_key.as_ref()).expect("key must be 32 bytes"));
    let seed_bytes = seed.encipher(None).expect("encipher source seed");
    let encrypted_master_seed = encrypt_integral_nonce(&main_cipher, MASTER_SEED_AAD, &seed_bytes);

    for (key, value) in [
        ("SecondaryKeyVersion", SECONDARY_KEY_VERSION.to_string()),
        ("SecondaryKeySalt", salt),
        ("SecondaryKeyHash", hex::encode(secondary_key.as_ref())),
        ("EncryptedMainKey", hex::encode(encrypted_main_key)),
        ("MasterSeed", hex::encode(encrypted_master_seed)),
    ] {
        conn.execute(
            "INSERT INTO wallet_settings (key, value) VALUES (?1, ?2)",
            params![key, value],
        )
        .expect("insert wallet setting");
    }
}

fn insert_completed_transaction(
    source_db_path: &Path,
    tx_id: i64,
    amount: i64,
    fee: i64,
    direction: i64,
    status: i64,
    mined_height: Option<i64>,
) {
    let conn = Connection::open(source_db_path).expect("open source db");
    let timestamp =
        chrono::NaiveDateTime::parse_from_str("2026-01-15 10:00:00", "%Y-%m-%d %H:%M:%S").expect("timestamp");
    let block_hash = mined_height.map(|height| fixed_hash_bytes((height & 0xff) as u8));

    conn.execute(
        r#"
        INSERT INTO completed_transactions (
            tx_id,
            source_address,
            destination_address,
            amount,
            fee,
            transaction_protocol,
            status,
            timestamp,
            cancelled,
            direction,
            mined_height,
            mined_in_block,
            payment_id,
            user_payment_id
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, ?9, ?10, ?11, NULL, NULL)
        "#,
        params![
            tx_id,
            Vec::<u8>::new(),
            Vec::<u8>::new(),
            amount,
            fee,
            Vec::<u8>::new(),
            status,
            timestamp,
            direction,
            mined_height,
            block_hash,
        ],
    )
    .expect("insert completed transaction");
}

fn insert_output(source_db_path: &Path, key_manager: &KeyManager, spec: OutputSpec) -> String {
    let conn = Connection::open(source_db_path).expect("open source db");
    let wallet_output = build_wallet_output(key_manager, spec.value);
    let output_hash = wallet_output.output_hash();
    let output_hash_hex = hex::encode(output_hash);

    let spending_key = if spec.legacy_spending_key {
        random_hex(32)
    } else {
        wallet_output.commitment_mask_key_id().to_string()
    };
    let script_private_key = if spec.legacy_script_key {
        random_hex(32)
    } else {
        wallet_output.script_key_id().to_string()
    };

    let mut covenant_bytes = Vec::new();
    BorshSerialize::serialize(wallet_output.covenant(), &mut covenant_bytes).expect("serialize covenant");

    conn.execute(
        r#"
        INSERT INTO outputs (
            commitment,
            rangeproof,
            spending_key,
            value,
            output_type,
            maturity,
            status,
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
        ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, 0, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21,
            ?22, ?23, ?24, ?25, ?26
        )
        "#,
        params![
            wallet_output.commitment().to_vec(),
            wallet_output.range_proof().as_ref().map(|proof| proof.to_vec()),
            spending_key,
            spec.value as i64,
            i64::from(wallet_output.features().output_type.as_byte()),
            wallet_output.features().maturity as i64,
            output_hash.to_vec(),
            wallet_output.script().to_bytes(),
            wallet_output.input_data().to_bytes(),
            script_private_key,
            wallet_output.script_lock_height() as i64,
            wallet_output.sender_offset_public_key().to_vec(),
            wallet_output.metadata_signature().ephemeral_commitment().to_vec(),
            wallet_output.metadata_signature().ephemeral_pubkey().to_vec(),
            wallet_output.metadata_signature().u_a().to_vec(),
            wallet_output.metadata_signature().u_x().to_vec(),
            wallet_output.metadata_signature().u_y().to_vec(),
            spec.mined_height,
            fixed_hash_bytes(spec.block_seed),
            spec.received_in_tx_id,
            serde_json::to_string(wallet_output.features()).expect("serialize features"),
            covenant_bytes,
            chrono::NaiveDateTime::parse_from_str("2026-01-15 10:00:00", "%Y-%m-%d %H:%M:%S").expect("timestamp"),
            wallet_output.encrypted_data().to_byte_vec(),
            wallet_output.minimum_value_promise().as_u64() as i64,
            None::<Vec<u8>>,
        ],
    )
    .expect("insert output");

    output_hash_hex
}

fn insert_scanned_block(source_db_path: &Path, height: u64, block_hash: Vec<u8>) {
    let conn = Connection::open(source_db_path).expect("open source db");
    let timestamp =
        chrono::NaiveDateTime::parse_from_str("2026-01-15 10:00:00", "%Y-%m-%d %H:%M:%S").expect("timestamp");
    conn.execute(
        "INSERT INTO scanned_blocks (header_hash, height, num_outputs, amount, timestamp) VALUES (?1, ?2, NULL, NULL, ?3)",
        params![block_hash, height as i64, timestamp],
    )
    .expect("insert scanned block");
}

fn insert_sync_tip(source_db_path: &Path, height: u64, block_hash: Vec<u8>) {
    let conn = Connection::open(source_db_path).expect("open source db");
    conn.execute(
        "INSERT INTO sync_tip (id, height, header_hash) VALUES (1, ?1, ?2)",
        params![height as i64, block_hash],
    )
    .expect("insert sync tip");
}

fn build_wallet_output(key_manager: &KeyManager, value: u64) -> WalletOutput {
    let (commitment_mask_key, script_key) = key_manager
        .get_next_commitment_mask_and_script_key()
        .expect("commitment mask and script key");
    let sender_offset = key_manager.get_random_key(None, None).expect("sender offset key");

    WalletOutputBuilder::new(MicroMinotari::from(value), commitment_mask_key.key_id.clone())
        .with_features(OutputFeatures::default())
        .with_script(script![Nop].expect("nop script"))
        .encrypt_data_for_recovery(key_manager, None, MemoField::new_empty())
        .expect("encrypt data")
        .with_input_data(inputs!(script_key.pub_key.clone()))
        .with_covenant(Covenant::default())
        .with_sender_offset_public_key(sender_offset.pub_key.clone())
        .with_script_key(script_key.key_id.clone())
        .sign_metadata_signature(key_manager, &sender_offset.key_id)
        .expect("sign metadata")
        .try_build(key_manager)
        .expect("build wallet output")
}

fn source_key_manager(seed: &CipherSeed) -> KeyManager {
    let seed_wallet = SeedWordsWallet::construct_new(seed.clone()).expect("seed wallet");
    let wallet = WalletType::SeedWords(seed_wallet);
    KeyManager::new(wallet).expect("key manager")
}

fn run_dry_run(ctx: &TestContext, allow_partial_import: bool) -> (MigrationReport, crate::db::SqlitePool) {
    let pool = init_db(ctx.dest_db_path.clone()).expect("init destination db");
    let report =
        run_migration_from_context_with_pool(ctx, &pool, true, allow_partial_import).expect("dry-run migration report");
    (report, pool)
}

fn run_live_migration(ctx: &TestContext, allow_partial_import: bool) -> (MigrationReport, crate::db::SqlitePool) {
    let pool = init_db(ctx.dest_db_path.clone()).expect("init destination db");
    let report =
        run_migration_from_context_with_pool(ctx, &pool, false, allow_partial_import).expect("live migration report");
    (report, pool)
}

fn run_migration_from_context(
    ctx: &TestContext,
    dry_run: bool,
    allow_partial_import: bool,
) -> Result<MigrationReport, anyhow::Error> {
    let pool = init_db(ctx.dest_db_path.clone()).expect("init destination db");
    run_migration_from_context_with_pool(ctx, &pool, dry_run, allow_partial_import)
}

fn run_migration_from_context_with_pool(
    ctx: &TestContext,
    pool: &crate::db::SqlitePool,
    dry_run: bool,
    allow_partial_import: bool,
) -> Result<MigrationReport, anyhow::Error> {
    let (console_db, cipher_seed) = ConsoleDb::open(&ctx.source_db_path, &ctx.source_passphrase)?;
    let key_manager = source_key_manager(&cipher_seed);
    let private_view_key = key_manager.get_private_view_key();

    run_migration(
        &console_db,
        &cipher_seed,
        pool,
        MigrationOptions {
            account_name: "imported",
            password: "destination-password",
            dry_run,
            allow_partial_import,
            account_view_key: &private_view_key,
        },
    )
}

fn random_hex(num_bytes: usize) -> String {
    let mut bytes = vec![0u8; num_bytes];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

fn fixed_hash_bytes(seed: u8) -> Vec<u8> {
    vec![seed; 32]
}

fn encrypt_integral_nonce(cipher: &XChaCha20Poly1305, aad: &[u8], plaintext: &[u8]) -> Vec<u8> {
    let mut nonce_bytes = [0u8; 24];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = XNonce::try_from(nonce_bytes.as_ref()).expect("nonce must be 24 bytes");
    let ciphertext = cipher
        .encrypt(&nonce, Payload { msg: plaintext, aad })
        .expect("encrypt");

    let mut combined = nonce_bytes.to_vec();
    combined.extend_from_slice(&ciphertext);
    combined
}
