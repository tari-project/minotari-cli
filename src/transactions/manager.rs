use anyhow::anyhow;
use chrono::{Duration, Utc};
use sqlx::{Pool, Sqlite, pool::PoolConnection};
use tari_common::configuration::Network;
use tari_common_types::{tari_address::TariAddressFeatures, transaction::TxId};
use tari_transaction_components::{
    MicroMinotari, TransactionBuilder,
    consensus::ConsensusConstantsBuilder,
    key_manager::KeyManager,
    offline_signing::{
        PaymentRecipient,
        models::{PrepareOneSidedTransactionForSigningResult, SignedOneSidedTransactionResult},
        prepare_one_sided_transaction_for_signing,
    },
    transaction_components::{MemoField, OutputFeatures, WalletOutput, memo_field::TxType},
};
use tari_utilities::ByteArray;

use crate::{
    db::{self, AccountRow},
    http::{TxSubmissionRejectionReason, WalletHttpClient},
    models::PendingTransactionStatus,
    transactions::{
        input_selector::{InputSelector, UtxoSelection},
        one_sided_transaction::Recipient,
    },
};

#[derive(Default)]
pub struct ProcessedTransaction {
    id: Option<String>,
    idempotency_key: String,
    recipient: Recipient,
    seconds_to_lock_utxos: u64,
    selected_utxos: Vec<WalletOutput>,
}

impl ProcessedTransaction {
    pub fn new(id: Option<String>, idempotency_key: String, recipient: Recipient, seconds_to_lock_utxos: u64) -> Self {
        Self {
            id,
            idempotency_key,
            recipient,
            seconds_to_lock_utxos,
            selected_utxos: Vec::new(),
        }
    }

    pub fn id(&self) -> &str {
        self.id.as_deref().unwrap_or("")
    }

    pub fn update_id(&mut self, id: String) {
        self.id = Some(id);
    }
}

pub struct TransactionSender {
    // Configuration
    pub db_pool: Pool<Sqlite>,
    pub network: Network,
    // Accpount
    pub account: AccountRow,
    pub password: String,

    pub processed_transactions: ProcessedTransaction,
    pub fee_per_gram: MicroMinotari,
}

impl TransactionSender {
    pub async fn new(
        db_pool: Pool<Sqlite>,
        account_name: String,
        password: String,
        network: Network,
    ) -> Result<Self, anyhow::Error> {
        let mut connection = db_pool.acquire().await?;
        let account_of_processed_transaction: AccountRow = db::get_account_by_name(&mut connection, &account_name)
            .await?
            .ok_or_else(|| anyhow!("Account with name '{}' not found", &account_name))?;

        Ok(Self {
            db_pool,
            network,
            account: account_of_processed_transaction,
            password,
            processed_transactions: ProcessedTransaction::default(),
            fee_per_gram: MicroMinotari(5),
        })
    }

    async fn get_connection(&self) -> Result<PoolConnection<Sqlite>, anyhow::Error> {
        self.db_pool
            .acquire()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to acquire database connection: {}", e))
    }

    async fn validate_transaction_creation_request(
        &mut self,
        processed_transaction: &ProcessedTransaction,
    ) -> Result<(), anyhow::Error> {
        let mut conn = self.db_pool.acquire().await?;
        if db::check_if_transaction_was_already_completed_by_idempotency_key(
            &mut conn,
            &processed_transaction.idempotency_key,
            self.account.id,
        )
        .await?
        {
            return Err(anyhow!(
                "A pending transaction with the same idempotency key already exists"
            ));
        }

        let sender_address = self.account.get_address(self.network, &self.password)?;
        if !sender_address
            .features()
            .contains(TariAddressFeatures::create_one_sided_only())
        {
            return Err(anyhow!("The sender address does not support one-sided transactions."));
        }

        Ok(())
    }

    async fn check_if_transaction_expired(
        &self,
        processed_transaction: &ProcessedTransaction,
    ) -> Result<(), anyhow::Error> {
        let mut conn = self.db_pool.acquire().await?;
        let is_expired = db::check_if_transaction_is_expired_by_idempotency_key(
            &mut conn,
            &processed_transaction.idempotency_key,
            self.account.id,
        )
        .await?;

        if is_expired {
            db::update_pending_transaction_status(
                &mut conn,
                processed_transaction.id(),
                PendingTransactionStatus::Expired,
            )
            .await?;
            return Err(anyhow!("The transaction has expired."));
        }

        Ok(())
    }

    async fn create_utxo_selection(
        &self,
        processed_transaction: &ProcessedTransaction,
    ) -> Result<UtxoSelection, anyhow::Error> {
        let mut connection = self.get_connection().await?;
        let amount = processed_transaction.recipient.amount;
        let num_outputs = 1;
        let estimated_output_size = None;

        let input_selector = InputSelector::new(self.account.id);
        let utxo_selection = input_selector
            .fetch_unspent_outputs(
                &mut connection,
                amount,
                num_outputs,
                self.fee_per_gram,
                estimated_output_size,
            )
            .await?;
        Ok(utxo_selection)
    }

    async fn create_pending_transaction(
        &self,
        processed_transaction: &mut ProcessedTransaction,
    ) -> Result<String, anyhow::Error> {
        let mut connection = self.get_connection().await?;

        let expires_at = Utc::now() + Duration::seconds(processed_transaction.seconds_to_lock_utxos as i64);
        let utxo_selection = self.create_utxo_selection(processed_transaction).await?;

        let pending_tx_id = db::create_pending_transaction(
            &mut connection,
            &processed_transaction.idempotency_key,
            self.account.id,
            utxo_selection.requires_change_output,
            utxo_selection.total_value,
            utxo_selection.fee_without_change,
            utxo_selection.fee_with_change,
            expires_at,
        )
        .await?;

        for utxo in &utxo_selection.utxos {
            db::lock_output(&mut connection, utxo.id, &pending_tx_id, expires_at).await?;
        }

        if processed_transaction.selected_utxos.is_empty() {
            let locked_utxos = utxo_selection
                .utxos
                .iter()
                .map(|db_utxo| db_utxo.output.clone())
                .collect();
            processed_transaction.selected_utxos = locked_utxos;
        }

        Ok(pending_tx_id)
    }

    async fn create_or_find_pending_transaction(
        &self,
        processed_transaction: &mut ProcessedTransaction,
    ) -> Result<String, anyhow::Error> {
        let mut connection = self.get_connection().await?;

        let response = db::find_pending_transaction_by_idempotency_key(
            &mut connection,
            &processed_transaction.idempotency_key,
            self.account.id,
        )
        .await?;
        if let Some(pending_tx) = response {
            Ok(pending_tx.id.to_string())
        } else {
            let pending_tx_id = self.create_pending_transaction(processed_transaction).await?;
            Ok(pending_tx_id)
        }
    }
    async fn prepare_transaction_builder(
        &self,
        locked_utxos: Vec<WalletOutput>,
    ) -> Result<TransactionBuilder<KeyManager>, anyhow::Error> {
        let key_manager = self.account.get_key_manager(&self.password).await?;
        let consensus_constants = ConsensusConstantsBuilder::new(self.network).build();
        let mut tx_builder = TransactionBuilder::new(consensus_constants, key_manager.clone(), self.network)?;

        tx_builder.with_fee_per_gram(self.fee_per_gram);

        for utxo in &locked_utxos {
            tx_builder.with_input(utxo.clone())?;
        }

        Ok(tx_builder)
    }

    pub async fn start_new_transaction(
        &mut self,
        idempotency_key: String,
        recipient: Recipient,
        seconds_to_lock_utxo: u64,
    ) -> Result<PrepareOneSidedTransactionForSigningResult, anyhow::Error> {
        let mut connection = self.get_connection().await?;

        let mut processed_transaction =
            ProcessedTransaction::new(None, idempotency_key, recipient.clone(), seconds_to_lock_utxo);

        self.validate_transaction_creation_request(&processed_transaction)
            .await?;

        let pending_transaction_id = self
            .create_or_find_pending_transaction(&mut processed_transaction)
            .await?;
        processed_transaction.update_id(pending_transaction_id.clone());

        let mut utxo_selection = processed_transaction.selected_utxos.clone();
        if utxo_selection.is_empty() {
            let db_utxo_selection =
                db::fetch_outputs_by_lock_request_id(&mut connection, processed_transaction.id()).await?;
            utxo_selection = db_utxo_selection.into_iter().map(|db_out| db_out.output).collect();
        }

        let tx_builder = self.prepare_transaction_builder(utxo_selection).await?;

        let sender_address = self.account.get_address(self.network, &self.password)?;
        let tx_id = TxId::new_random();

        let payment_id = match &recipient.payment_id {
            Some(s) => MemoField::new_open_from_string(s, TxType::PaymentToOther).map_err(|e| anyhow!(e))?,
            None => MemoField::new_empty(),
        };
        let output_features = OutputFeatures::default();

        let payment_recipient = PaymentRecipient {
            amount: recipient.amount,
            output_features: output_features.clone(),
            address: recipient.address.clone(),
            payment_id: payment_id.clone(),
        };

        let result = tokio::task::spawn_blocking(move || {
            prepare_one_sided_transaction_for_signing(
                tx_id,
                tx_builder,
                &[payment_recipient],
                payment_id,
                sender_address,
            )
        })
        .await??;

        self.processed_transactions = processed_transaction;

        Ok(result)
    }

    pub async fn finalize_trasaction_and_broadcast(
        &self,
        signed_transaction: SignedOneSidedTransactionResult,
        grpc_address: String,
    ) -> Result<(), anyhow::Error> {
        let mut connection = self.get_connection().await?;
        let processed_transaction = &self.processed_transactions;
        let account_id = self.account.id;

        self.check_if_transaction_expired(processed_transaction).await?;

        let kernel_excess = signed_transaction
            .signed_transaction
            .transaction
            .body()
            .kernels()
            .first()
            .map(|k| k.excess.as_bytes().to_vec())
            .unwrap_or_default();

        let serialized_transaction = serde_json::to_vec(&signed_transaction.signed_transaction.transaction)
            .map_err(|e| anyhow!("Failed to serialize transaction: {}", e))?;

        let sent_payref = signed_transaction
            .signed_transaction
            .sent_hashes
            .first()
            .map(hex::encode);

        db::update_pending_transaction_status(
            &mut connection,
            processed_transaction.id(),
            crate::models::PendingTransactionStatus::Completed,
        )
        .await?;

        let completed_tx_id = db::create_completed_transaction(
            &mut connection,
            account_id,
            processed_transaction.id(),
            &kernel_excess,
            &serialized_transaction,
            sent_payref,
        )
        .await?;

        let wallet_http_client = WalletHttpClient::new(grpc_address.parse()?)?;

        let response = wallet_http_client
            .submit_transaction(signed_transaction.signed_transaction.transaction)
            .await;

        match response {
            Err(e) => {
                db::mark_completed_transaction_as_rejected(
                    &mut connection,
                    &completed_tx_id,
                    &format!("Transaction submission failed: {}", e),
                )
                .await?;

                return Err(anyhow!("Transaction submission failed: {}", e));
            },
            Ok(response) => {
                if response.accepted {
                    db::mark_completed_transaction_as_broadcasted(&mut connection, &completed_tx_id, 1).await?;
                } else if !response.accepted && response.rejection_reason != TxSubmissionRejectionReason::AlreadyMined {
                    db::mark_completed_transaction_as_rejected(
                        &mut connection,
                        &completed_tx_id,
                        &response.rejection_reason.to_string(),
                    )
                    .await?;

                    return Err(anyhow!(
                        "Transaction was not accepted by the network: {}",
                        response.rejection_reason
                    ));
                }
            },
        }

        Ok(())
    }
}
