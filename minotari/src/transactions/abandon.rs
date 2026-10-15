//! Abandoning a send the base node has not accepted.
//!
//! The node's answers never reject a send (see [`crate::transactions::monitor`]):
//! a send it keeps refusing stays pending, its inputs locked, and is rebroadcast
//! less often once stalled. [`abandon_send`] lets the user give such a send up
//! and get its inputs back.
//!
//! Abandoning does not unsend anything. If a node still holds the signed
//! transaction it can still be mined; the wallet's scan then finds its change
//! output and marks it mined (`MinedUnconfirmed`) after all.

use log::warn;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior};
use tari_common_types::transaction::TxId;
use tari_transaction_components::transaction_components::Transaction;
use thiserror::Error;

use crate::{
    db::{self, CompletedTransactionStatus, WalletDbError},
    models::{WalletEvent, WalletEventType},
    transactions::{DisplayedTransaction, monitor::MAX_BROADCAST_ATTEMPTS},
    webhooks::{WebhookTriggerConfig, utils::trigger_webhook_with_balance},
};

/// The rejection reason recorded for an abandoned send.
pub const ABANDONED_REASON: &str = "abandoned by user";

/// Why a send cannot be abandoned.
#[derive(Debug, Error)]
pub enum AbandonSendError {
    #[error("No send {0} in this account")]
    NotFound(TxId),
    #[error("Send {tx_id} is {status}; only a send that is not mined yet (completed or broadcast) can be abandoned")]
    NotPending { tx_id: TxId, status: String },
    #[error(
        "Send {tx_id} is not stalled ({attempts} broadcasts in a row not accepted, stalled at {MAX_BROADCAST_ATTEMPTS}); \
         pass force to abandon it anyway"
    )]
    NotStalled { tx_id: TxId, attempts: i32 },
    #[error("An input of send {tx_id} has already been spent on chain; the wallet's scan settles it")]
    InputSpent { tx_id: TxId },
    #[error("The change of send {tx_id} is an input of another send ({by}); abandon or settle that send first")]
    ChangeInUse { tx_id: TxId, by: String },
    #[error(transparent)]
    Db(#[from] WalletDbError),
}

impl From<rusqlite::Error> for AbandonSendError {
    fn from(e: rusqlite::Error) -> Self {
        AbandonSendError::Db(WalletDbError::from(e))
    }
}

/// What abandoning a send did.
#[derive(Debug, Clone)]
pub struct AbandonedSend {
    /// The persisted `TransactionRejected` event.
    pub event: WalletEvent,
    /// The send's displayed transaction, now rejected, if it has one.
    pub displayed_transaction: Option<DisplayedTransaction>,
}

/// Gives up a send the node has not accepted, and hands its inputs back.
///
/// Only a send that is `Completed` or `Broadcast` can be abandoned. Unless
/// `force` is set it must also be stalled: the node has not accepted it
/// [`MAX_BROADCAST_ATTEMPTS`] times in a row. A send any of whose inputs has
/// already been seen spent on chain is refused: the scan settles it. So is a
/// send whose change output is an input of another send.
///
/// In one write transaction: the send is marked `Rejected` ("abandoned by
/// user"), its change output `REJECTED` (kept, so the scan can still recognise
/// the send if it is mined after all), its inputs still locked are released,
/// its displayed transaction rejected, and a `TransactionRejected` event
/// persisted (with its webhook when `webhook_config` is given).
///
/// The original transaction can still be mined if a node holds it; the scan
/// then marks it `MinedUnconfirmed`. A released input spent again in the
/// meantime then loses to it.
pub fn abandon_send(
    conn: &mut Connection,
    account_id: i64,
    tx_id: TxId,
    force: bool,
    webhook_config: Option<&WebhookTriggerConfig>,
) -> Result<AbandonedSend, AbandonSendError> {
    let db_tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

    let send = match db::get_completed_transaction_by_id(&db_tx, tx_id)? {
        Some(send) if send.account_id == account_id => send,
        _ => return Err(AbandonSendError::NotFound(tx_id)),
    };
    if !matches!(
        send.status,
        CompletedTransactionStatus::Completed | CompletedTransactionStatus::Broadcast
    ) {
        return Err(AbandonSendError::NotPending {
            tx_id,
            status: send.status.to_string(),
        });
    }
    if !force && send.broadcast_attempts < MAX_BROADCAST_ATTEMPTS {
        return Err(AbandonSendError::NotStalled {
            tx_id,
            attempts: send.broadcast_attempts,
        });
    }

    // Refuse if the chain already spent any of its inputs: then the send either
    // landed or lost to another transaction, and the scan decides which.
    if let Ok(transaction) = serde_json::from_slice::<Transaction>(&send.serialized_transaction) {
        for input in transaction.body().inputs() {
            if db::get_output_spend(&db_tx, account_id, &input.output_hash())?.is_some() {
                return Err(AbandonSendError::InputSpent { tx_id });
            }
        }
    }
    let spent_inputs: i64 = db_tx.query_row(
        r#"
        SELECT COUNT(*) FROM outputs o
        WHERE o.locked_by_request_id = :req AND o.status = 'SPENT' AND o.deleted_at IS NULL
          AND EXISTS (SELECT 1 FROM inputs i WHERE i.output_id = o.id AND i.deleted_at IS NULL)
        "#,
        rusqlite::named_params! { ":req": send.pending_tx_id },
        |row| row.get(0),
    )?;
    if spent_inputs > 0 {
        return Err(AbandonSendError::InputSpent { tx_id });
    }

    // Its change may be an input of a later send (a reorg made it unconfirmed
    // again). Rejecting it would strand that send, and its lock with it.
    let change_lock: Option<Option<String>> = db_tx
        .query_row(
            "SELECT locked_by_request_id FROM outputs WHERE created_by_request_id = :req AND deleted_at IS NULL",
            rusqlite::named_params! { ":req": send.pending_tx_id },
            |row| row.get(0),
        )
        .optional()?;
    if let Some(Some(by)) = change_lock
        && by != send.pending_tx_id
    {
        return Err(AbandonSendError::ChangeInUse { tx_id, by });
    }

    warn!(
        target: "audit",
        id = tx_id.to_string().as_str();
        "Send abandoned by user; its inputs are released"
    );
    db::mark_completed_transaction_as_rejected(&db_tx, tx_id, ABANDONED_REASON)?;
    // Before the release, which soft-deletes an `UNCONFIRMED` change: the change
    // is kept, `REJECTED`, so the scan can still recognise the send by it.
    db::mark_change_output_rejected(&db_tx, &send.pending_tx_id)?;
    db::unlock_outputs_for_request(&db_tx, &send.pending_tx_id)?;
    // An input a reorg un-spent (its spend record removed) is still marked
    // `SPENT`; it is not spent on chain, so it is handed back too.
    db_tx.execute(
        r#"
        UPDATE outputs
        SET status = 'UNSPENT', locked_by_request_id = NULL, locked_at = NULL
        WHERE locked_by_request_id = :req AND status = 'SPENT' AND deleted_at IS NULL
          AND NOT EXISTS (SELECT 1 FROM inputs i WHERE i.output_id = outputs.id AND i.deleted_at IS NULL)
        "#,
        rusqlite::named_params! { ":req": send.pending_tx_id },
    )?;
    let displayed_transaction = db::mark_displayed_transaction_rejected(&db_tx, tx_id)?;

    let mut event = WalletEvent {
        id: 0,
        account_id,
        event_type: WalletEventType::TransactionRejected {
            tx_id,
            reason: ABANDONED_REASON.to_string(),
        },
        description: format!("Transaction {} abandoned by user", tx_id),
    };
    let event_id = db::insert_wallet_event(&db_tx, account_id, &event)?;
    if let Some(config) = webhook_config {
        trigger_webhook_with_balance(&db_tx, account_id, event_id, &event, config)?;
    }
    event.id = event_id;

    db_tx.commit()?;
    Ok(AbandonedSend {
        event,
        displayed_transaction,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{SqlitePool, create_account, get_account_by_name, init_db};
    use crate::transactions::idempotency::{IdempotencyBinding, IdempotencyOperation, RequestFingerprint};
    use crate::transactions::{
        BlockchainInfo, TransactionDetails, TransactionDirection, TransactionDisplayStatus, TransactionSource,
    };
    use chrono::Utc;
    use rusqlite::named_params;
    use tari_common_types::seeds::cipher_seed::CipherSeed;
    use tari_common_types::types::{ComAndPubSignature, CompressedPublicKey, FixedHash, PrivateKey};
    use tari_script::{ExecutionStack, TariScript};
    use tari_transaction_components::MicroMinotari;
    use tari_transaction_components::key_manager::wallet_types::{SeedWordsWallet, WalletType};
    use tari_transaction_components::key_manager::{KeyManager, TariKeyId, TransactionKeyManagerInterface};
    use tari_transaction_components::transaction_components::{
        EncryptedData, MemoField, OutputFeatures, TransactionInput, TransactionOutputVersion, WalletOutput,
        covenants::Covenant,
    };
    use tempfile::TempDir;

    fn test_wallet_output(value: u64, hash_byte: u8) -> WalletOutput {
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
            FixedHash::from([hash_byte; 32]),
            Default::default(),
        )
    }

    struct Send {
        pool: SqlitePool,
        account_id: i64,
        tx_id: TxId,
        request_id: String,
        input_id: i64,
        _dir: TempDir,
    }

    /// A broadcast send that the node has not accepted `attempts` times in a
    /// row: one input locked to it, an `UNCONFIRMED` change, a pending entry.
    fn send_with_attempts(attempts: i32) -> Send {
        let dir = tempfile::tempdir().expect("temp dir");
        let pool = init_db(dir.path().join("abandon.db")).expect("init db");
        let conn = pool.get().expect("conn");
        let wallet = WalletType::SeedWords(SeedWordsWallet::construct_new(CipherSeed::random()).unwrap());
        create_account(&conn, "default", &wallet, "password").unwrap();
        let account_id = get_account_by_name(&conn, "default").unwrap().unwrap().id;
        let view_key = KeyManager::new(wallet).unwrap().get_private_view_key();

        let binding = IdempotencyBinding::new(
            Some("send".to_string()),
            IdempotencyOperation::SendTransaction,
            RequestFingerprint::new(IdempotencyOperation::SendTransaction),
        );
        let request_id = db::create_pending_transaction(
            &conn,
            "send",
            &binding,
            account_id,
            true,
            MicroMinotari(0),
            MicroMinotari(0),
            MicroMinotari(0),
            Utc::now() + chrono::Duration::hours(1),
        )
        .expect("pending");
        let input = test_wallet_output(10_000, 1);
        let input_id = db::insert_output(
            &conn,
            account_id,
            &view_key,
            input.output_hash().to_vec(),
            &input,
            10,
            &FixedHash::from([10u8; 32]),
            1_700_000_000,
            None,
            None,
            FixedHash::zero(),
            false,
        )
        .expect("input");
        db::lock_output(&conn, input_id, &request_id, Utc::now()).expect("lock");
        let transaction = Transaction::new(
            vec![TransactionInput::new_with_output_hash(
                input.output_hash(),
                ExecutionStack::default(),
                ComAndPubSignature::default(),
            )],
            Vec::new(),
            Vec::new(),
            PrivateKey::default(),
            PrivateKey::default(),
        );
        let tx_id = TxId::from(1u64);
        db::create_completed_transaction(
            &conn,
            account_id,
            &request_id,
            &[],
            &serde_json::to_vec(&transaction).expect("serialize"),
            None,
            tx_id,
        )
        .expect("completed");
        db::mark_completed_transaction_as_broadcasted(&conn, tx_id, attempts).expect("broadcast");
        db::insert_unconfirmed_change_output(&conn, account_id, &view_key, &request_id, &test_wallet_output(3_000, 3))
            .expect("change");
        db::insert_displayed_transaction(
            &conn,
            &DisplayedTransaction {
                id: tx_id,
                direction: TransactionDirection::Outgoing,
                source: TransactionSource::OneSided,
                status: TransactionDisplayStatus::Pending,
                amount: MicroMinotari::from(1_000),
                message: None,
                counterparty: None,
                blockchain: BlockchainInfo {
                    block_height: 0,
                    timestamp: Utc::now().naive_utc(),
                    confirmations: 0,
                    block_hash: FixedHash::zero(),
                },
                fee: None,
                details: TransactionDetails {
                    account_id,
                    total_credit: MicroMinotari::from(0),
                    total_debit: MicroMinotari::from(1_000),
                    inputs: Vec::new(),
                    outputs: Vec::new(),
                    output_type: None,
                    coinbase_extra: None,
                    memo_hex: None,
                    sent_output_hashes: Vec::new(),
                    sent_payrefs: Vec::new(),
                },
                lock_height: 0,
            },
        )
        .expect("displayed");
        drop(conn);
        Send {
            pool,
            account_id,
            tx_id,
            request_id,
            input_id,
            _dir: dir,
        }
    }

    fn abandon(send: &Send, force: bool) -> Result<AbandonedSend, AbandonSendError> {
        abandon_send(
            &mut send.pool.get().expect("conn"),
            send.account_id,
            send.tx_id,
            force,
            None,
        )
    }

    fn status_of(send: &Send) -> CompletedTransactionStatus {
        let conn = send.pool.get().expect("conn");
        db::get_completed_transaction_by_id(&conn, send.tx_id)
            .expect("query")
            .expect("exists")
            .status
    }

    /// The input's status and lock, and the change's status and whether it is active.
    fn rows(send: &Send) -> ((String, Option<String>), (String, bool)) {
        let conn = send.pool.get().expect("conn");
        let input = conn
            .query_row(
                "SELECT status, locked_by_request_id FROM outputs WHERE id = :id",
                named_params! { ":id": send.input_id },
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("input");
        let change = conn
            .query_row(
                "SELECT status, deleted_at IS NULL FROM outputs WHERE created_by_request_id = :req",
                named_params! { ":req": send.request_id },
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("change");
        (input, change)
    }

    #[test]
    fn a_stalled_send_is_abandoned_and_its_inputs_released() {
        let send = send_with_attempts(MAX_BROADCAST_ATTEMPTS);

        let abandoned = abandon(&send, false).expect("abandon");

        assert_eq!(status_of(&send), CompletedTransactionStatus::Rejected);
        let conn = send.pool.get().expect("conn");
        let completed = db::get_completed_transaction_by_id(&conn, send.tx_id).unwrap().unwrap();
        assert_eq!(completed.last_rejected_reason.as_deref(), Some(ABANDONED_REASON));
        assert_eq!(completed.mined_height, None);
        let ((input_status, input_lock), (change_status, change_active)) = rows(&send);
        assert_eq!(input_status, "UNSPENT");
        assert_eq!(input_lock, None);
        assert_eq!(change_status, "REJECTED");
        assert!(change_active, "the change is kept so the scan can still find the send");
        assert_eq!(
            abandoned.displayed_transaction.map(|d| d.status),
            Some(TransactionDisplayStatus::Rejected)
        );
        assert!(matches!(
            &abandoned.event.event_type,
            WalletEventType::TransactionRejected { reason, .. } if reason == ABANDONED_REASON
        ));
        let stored = db::get_events_by_account_id(&conn, send.account_id, 10, 0).expect("events");
        assert!(stored.iter().any(|e| e.event_type == "TransactionRejected"));
    }

    #[test]
    fn a_send_that_is_not_stalled_needs_force() {
        let send = send_with_attempts(1);
        assert!(matches!(
            abandon(&send, false),
            Err(AbandonSendError::NotStalled { attempts: 1, .. })
        ));
        assert_eq!(status_of(&send), CompletedTransactionStatus::Broadcast);
        assert_eq!(rows(&send).0.0, "LOCKED");

        abandon(&send, true).expect("abandon with force");
        assert_eq!(status_of(&send), CompletedTransactionStatus::Rejected);
    }

    #[test]
    fn a_send_whose_input_is_spent_on_chain_cannot_be_abandoned() {
        let send = send_with_attempts(MAX_BROADCAST_ATTEMPTS);
        {
            let conn = send.pool.get().expect("conn");
            db::insert_input(&conn, send.account_id, send.input_id, 50, &[5u8; 32], 1_700_000_000).expect("spend");
            db::update_output_status(&conn, send.input_id, crate::models::OutputStatus::Spent).expect("spent");
        }
        assert!(matches!(abandon(&send, true), Err(AbandonSendError::InputSpent { .. })));
        assert_eq!(status_of(&send), CompletedTransactionStatus::Broadcast);
    }

    #[test]
    fn a_mined_or_rejected_send_cannot_be_abandoned() {
        let send = send_with_attempts(MAX_BROADCAST_ATTEMPTS);
        {
            let conn = send.pool.get().expect("conn");
            db::mark_completed_transaction_as_mined_unconfirmed(&conn, send.tx_id, 50, &[5u8; 32]).expect("mined");
        }
        assert!(matches!(abandon(&send, true), Err(AbandonSendError::NotPending { .. })));

        let rejected = send_with_attempts(MAX_BROADCAST_ATTEMPTS);
        abandon(&rejected, false).expect("abandon");
        assert!(matches!(
            abandon(&rejected, true),
            Err(AbandonSendError::NotPending { .. })
        ));
    }

    #[test]
    fn another_accounts_send_is_not_found() {
        let send = send_with_attempts(MAX_BROADCAST_ATTEMPTS);
        assert!(matches!(
            abandon_send(
                &mut send.pool.get().expect("conn"),
                send.account_id + 1,
                send.tx_id,
                true,
                None
            ),
            Err(AbandonSendError::NotFound(_))
        ));
    }
}
