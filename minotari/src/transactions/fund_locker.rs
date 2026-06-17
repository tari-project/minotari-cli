//! UTXO locking mechanism for transaction construction.
//!
//! This module provides functionality to temporarily lock UTXOs (Unspent Transaction Outputs)
//! during transaction construction, preventing double-spending scenarios where the same
//! outputs might be selected for multiple concurrent transactions.
//!
//! # Overview
//!
//! When creating a transaction, the wallet must:
//! 1. Select appropriate UTXOs to cover the transaction amount plus fees
//! 2. Lock those UTXOs to prevent other transactions from using them
//! 3. Either complete the transaction (consuming the UTXOs) or release the lock on failure
//!
//! The [`FundLocker`] handles steps 1 and 2, with automatic expiration to handle step 3
//! in case of failures or timeouts.
//!
//! # Idempotency
//!
//! Lock operations support idempotency keys, allowing clients to safely retry requests
//! without accidentally locking additional funds. If a lock request with the same
//! idempotency key already exists, the original result is returned.

use chrono::{Duration, Utc};
use log::info;
use rusqlite::TransactionBehavior;
use tari_transaction_components::tari_amount::MicroMinotari;
use uuid::Uuid;

use crate::{
    api::types::LockFundsResult,
    db::{self, SqlitePool},
    log::mask_amount,
    transactions::input_selector::InputSelector,
};

/// Manages temporary locking of UTXOs during transaction construction.
///
/// `FundLocker` ensures that UTXOs selected for a transaction cannot be used
/// by other concurrent transactions, preventing double-spending within the wallet.
/// Locks are time-limited and automatically expire if the transaction is not
/// completed within the specified duration.
///
/// # Thread Safety
///
/// `FundLocker` uses database-level locking and can be safely shared across
/// threads via cloning (which clones the underlying connection pool).
///
/// # Example
///
/// ```rust,ignore
/// use minotari::transactions::fund_locker::FundLocker;
/// use tari_transaction_components::tari_amount::MicroMinotari;
///
/// let locker = FundLocker::new(db_pool);
///
/// // Lock funds for a transaction
/// let result = locker.lock(
///     account_id,
///     MicroMinotari(1_000_000),  // amount to send
///     1,                         // number of outputs
///     MicroMinotari(5),          // fee per gram
///     None,                      // use default output size estimate
///     Some("unique-key".into()), // idempotency key
///     300,                       // lock for 5 minutes
/// ).await?;
///
/// // Use result.utxos to build the transaction
/// ```
pub struct FundLocker {
    db_pool: SqlitePool,
}

impl FundLocker {
    /// Creates a new `FundLocker` with the given database connection pool.
    ///
    /// # Arguments
    ///
    /// * `db_pool` - SQLite connection pool for database operations
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let locker = FundLocker::new(db_pool);
    /// ```
    pub fn new(db_pool: SqlitePool) -> Self {
        Self { db_pool }
    }

    /// Locks UTXOs for a pending transaction.
    ///
    /// Selects unspent outputs sufficient to cover the requested amount plus estimated
    /// transaction fees, then locks them in the database with an expiration time.
    /// If an idempotency key is provided and a matching pending transaction exists,
    /// returns the existing lock result without creating a new one.
    ///
    /// # Arguments
    ///
    /// * `account_id` - The account whose UTXOs should be locked
    /// * `amount` - The amount to be sent (excluding fees)
    /// * `num_outputs` - Number of transaction outputs (typically 1 for recipient + optional change)
    /// * `fee_per_gram` - Fee rate in MicroMinotari per gram of transaction weight
    /// * `estimated_output_size` - Optional override for output size estimation; if `None`,
    ///   uses default calculation based on standard output features
    /// * `idempotency_key` - Optional unique key for idempotent operations; if provided and
    ///   a matching lock exists, returns the existing result
    /// * `seconds_to_lock_utxos` - Duration in seconds before the lock expires
    ///
    /// # Returns
    ///
    /// Returns a [`LockFundsResult`] containing:
    /// - The selected UTXOs
    /// - Whether a change output is required
    /// - Total value of selected UTXOs
    /// - Fee calculations with and without change output
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Database connection fails
    /// - Insufficient funds are available
    /// - UTXO selection fails due to serialization errors
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let result = locker.lock(
    ///     account_id,
    ///     MicroMinotari(500_000),
    ///     1,
    ///     MicroMinotari(5),
    ///     None,
    ///     Some("tx-123".to_string()),
    ///     600, // 10 minute lock
    /// ).await?;
    ///
    /// println!("Locked {} UTXOs worth {}", result.utxos.len(), result.total_value);
    /// ```
    #[allow(clippy::too_many_arguments)]
    pub fn lock(
        &self,
        account_id: i64,
        amount: MicroMinotari,
        num_outputs: usize,
        fee_per_gram: MicroMinotari,
        estimated_output_size: Option<usize>,
        idempotency_key: Option<String>,
        seconds_to_lock_utxos: u64,
        confirmation_window: u64,
    ) -> Result<LockFundsResult, anyhow::Error> {
        info!(
            target: "audit",
            account_id = account_id,
            amount = &*mask_amount(amount);
            "Locking funds"
        );
        let mut conn = self.db_pool.get()?;
        let transaction = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(idempotency_key_str) = &idempotency_key
            && let Some(response) = db::find_pending_transaction_locked_funds_by_idempotency_key(
                &transaction,
                idempotency_key_str,
                account_id,
            )?
        {
            info!(
                target: "audit",
                idempotency_key = idempotency_key_str.as_str();
                "Found existing pending transaction lock"
            );
            return Ok(response);
        }

        let input_selector = InputSelector::new(account_id, confirmation_window);
        let utxo_selection = input_selector.fetch_unspent_outputs(
            &transaction,
            amount,
            num_outputs,
            fee_per_gram,
            estimated_output_size,
        )?;

        #[allow(clippy::cast_possible_wrap)]
        let expires_at = Utc::now() + Duration::seconds(seconds_to_lock_utxos as i64);
        let idempotency_key = idempotency_key.unwrap_or_else(|| Uuid::new_v4().to_string());
        let pending_tx_id = db::create_pending_transaction(
            &transaction,
            &idempotency_key,
            account_id,
            utxo_selection.requires_change_output,
            utxo_selection.total_value,
            utxo_selection.fee_without_change,
            utxo_selection.fee_with_change,
            expires_at,
        )?;

        for utxo in &utxo_selection.utxos {
            db::lock_output(&transaction, utxo.id, &pending_tx_id, expires_at)?;
        }

        transaction.commit()?;

        info!(
            target: "audit",
            utxos_count = utxo_selection.utxos.len(),
            total_value = &*mask_amount(utxo_selection.total_value);
            "Funds locked successfully"
        );

        Ok(LockFundsResult {
            utxos: utxo_selection.utxos.iter().map(|utxo| utxo.output.clone()).collect(),
            requires_change_output: utxo_selection.requires_change_output,
            total_value: utxo_selection.total_value,
            fee_without_change: utxo_selection.fee_without_change,
            fee_with_change: utxo_selection.fee_with_change,
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::cast_possible_wrap)]
    #![allow(clippy::indexing_slicing)]

    use super::*;
    use crate::db::{create_account, get_account_by_name, init_db, insert_scanned_tip_block};
    use rusqlite::{Connection, named_params};
    use std::{
        collections::HashSet,
        sync::{Arc, Barrier},
        thread,
    };
    use tari_common_types::{
        seeds::cipher_seed::CipherSeed,
        types::{ComAndPubSignature, CompressedCommitment, CompressedPublicKey, FixedHash},
    };
    use tari_script::{ExecutionStack, TariScript};
    use tari_transaction_components::{
        key_manager::{
            TariKeyId,
            wallet_types::{SeedWordsWallet, WalletType},
        },
        transaction_components::{
            EncryptedData, MemoField, OutputFeatures, TransactionOutputVersion, WalletOutput, covenants::Covenant,
        },
    };
    use tempfile::{TempDir, tempdir};

    fn test_wallet_output(value: u64, output_hash_byte: u8) -> WalletOutput {
        let mut output_hash = FixedHash::default();
        output_hash[0] = output_hash_byte;
        WalletOutput::new_from_parts(
            TransactionOutputVersion::default(),
            MicroMinotari::from(value),
            TariKeyId::default(),
            OutputFeatures::default(),
            TariScript::default(),
            ExecutionStack::default(),
            TariKeyId::default(),
            CompressedPublicKey::default(),
            ComAndPubSignature::default(),
            0,
            Covenant::default(),
            EncryptedData::default(),
            MicroMinotari::from(0),
            None,
            MemoField::new_empty(),
            output_hash,
            CompressedCommitment::default(),
        )
    }

    fn insert_test_output(conn: &Connection, account_id: i64, output_hash_byte: u8, value: u64, mined_height: u64) {
        let output = test_wallet_output(value, output_hash_byte);
        let wallet_output_json = serde_json::to_string(&output).expect("serialize wallet output");
        conn.execute(
            r#"
            INSERT INTO outputs (
                account_id, tx_id, output_hash, mined_in_block_height, mined_in_block_hash,
                value, mined_timestamp, wallet_output_json, status, confirmed_height,
                confirmed_hash, is_burn, maturity
            ) VALUES (
                :account_id, :tx_id, :output_hash, :height, :block_hash,
                :value, :mined_ts, :json, :status, :confirmed_height,
                :confirmed_hash, 0, :maturity
            )
            "#,
            named_params! {
                ":account_id": account_id,
                ":tx_id": output_hash_byte as i64,
                ":output_hash": vec![output_hash_byte; 32],
                ":height": mined_height as i64,
                ":block_hash": vec![output_hash_byte; 32],
                ":value": value as i64,
                ":mined_ts": Utc::now(),
                ":json": wallet_output_json,
                ":status": "UNSPENT",
                ":confirmed_height": mined_height as i64,
                ":confirmed_hash": vec![output_hash_byte; 32],
                ":maturity": 0,
            },
        )
        .expect("insert test output");
    }

    fn setup_test_env(output_count: usize, value_per_output: u64) -> (SqlitePool, i64, TempDir) {
        let temp = tempdir().expect("temp dir");
        let pool = init_db(temp.path().join("wallet.db")).expect("init db");
        let conn = pool.get().expect("conn");

        let seeds = CipherSeed::random();
        let wallet = WalletType::SeedWords(SeedWordsWallet::construct_new(seeds).expect("construct wallet"));
        create_account(&conn, "default", &wallet, "test-password").expect("create account");
        let account_id = get_account_by_name(&conn, "default")
            .expect("get account")
            .expect("account exists")
            .id;

        insert_scanned_tip_block(&conn, account_id, 200, &[0u8; 32]).expect("insert tip block");
        for i in 0..output_count {
            insert_test_output(&conn, account_id, (i + 1) as u8, value_per_output, 100);
        }
        drop(conn);

        (pool, account_id, temp)
    }

    fn lock_funds(pool: SqlitePool, account_id: i64, idempotency_key: Option<String>) -> LockFundsResult {
        FundLocker::new(pool)
            .lock(
                account_id,
                MicroMinotari::from(100_000),
                1,
                MicroMinotari::from(0),
                Some(1000),
                idempotency_key,
                3600,
                100,
            )
            .expect("lock funds")
    }

    fn output_hashes(result: &LockFundsResult) -> HashSet<FixedHash> {
        result.utxos.iter().map(|utxo| utxo.output_hash()).collect()
    }

    #[test]
    fn concurrent_lock_calls_select_disjoint_utxos() {
        let (pool, account_id, _temp) = setup_test_env(4, 500_000);
        let barrier = Arc::new(Barrier::new(2));

        let pool_a = pool.clone();
        let barrier_a = barrier.clone();
        let handle_a = thread::spawn(move || {
            barrier_a.wait();
            lock_funds(pool_a, account_id, None)
        });

        let pool_b = pool.clone();
        let barrier_b = barrier.clone();
        let handle_b = thread::spawn(move || {
            barrier_b.wait();
            lock_funds(pool_b, account_id, None)
        });

        let result_a = handle_a.join().expect("thread A panicked");
        let result_b = handle_b.join().expect("thread B panicked");
        let hashes_a = output_hashes(&result_a);
        let hashes_b = output_hashes(&result_b);

        assert!(!hashes_a.is_empty(), "first call selected UTXOs");
        assert!(!hashes_b.is_empty(), "second call selected UTXOs");
        assert!(
            hashes_a.is_disjoint(&hashes_b),
            "concurrent lock calls must not select overlapping UTXOs"
        );
    }

    #[test]
    fn concurrent_idempotent_lock_returns_existing_request() {
        let (pool, account_id, _temp) = setup_test_env(4, 500_000);
        let key = "concurrent-idempotency-key".to_string();
        let barrier = Arc::new(Barrier::new(2));

        let pool_a = pool.clone();
        let key_a = key.clone();
        let barrier_a = barrier.clone();
        let handle_a = thread::spawn(move || {
            barrier_a.wait();
            lock_funds(pool_a, account_id, Some(key_a))
        });

        let pool_b = pool.clone();
        let key_b = key.clone();
        let barrier_b = barrier.clone();
        let handle_b = thread::spawn(move || {
            barrier_b.wait();
            lock_funds(pool_b, account_id, Some(key_b))
        });

        let result_a = handle_a.join().expect("thread A panicked");
        let result_b = handle_b.join().expect("thread B panicked");

        assert_eq!(output_hashes(&result_a), output_hashes(&result_b));
        assert_eq!(result_a.total_value, result_b.total_value);
        assert_eq!(result_a.fee_without_change, result_b.fee_without_change);
        assert_eq!(result_a.fee_with_change, result_b.fee_with_change);

        let conn = pool.get().expect("conn");
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pending_transactions WHERE idempotency_key = :key",
                named_params! { ":key": key },
                |row| row.get(0),
            )
            .expect("count pending transactions");
        assert_eq!(count, 1, "same idempotency key must create one pending transaction");
    }
}
