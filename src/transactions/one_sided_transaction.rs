use crate::{api::types::LockFundsResponse, db::AccountRow};
use anyhow::anyhow;
use sqlx::SqlitePool;
use tari_common::configuration::Network;
use tari_common_types::{tari_address::TariAddress, transaction::TxId};
use tari_transaction_components::offline_signing::models::PaymentRecipient;
use tari_transaction_components::{
    TransactionBuilder,
    consensus::ConsensusConstantsBuilder,
    offline_signing::{models::PrepareOneSidedTransactionForSigningResult, prepare_one_sided_transaction_for_signing},
    tari_amount::MicroMinotari,
    transaction_components::{MemoField, OutputFeatures, memo_field::TxType},
};

#[derive(Debug, Clone, Default)]
pub struct Recipient {
    pub address: TariAddress,
    pub amount: MicroMinotari,
    pub payment_id: Option<String>,
}

pub struct OneSidedTransaction {
    pub db_pool: SqlitePool,
    pub network: Network,
    pub password: String,
}

impl OneSidedTransaction {
    pub fn new(db_pool: SqlitePool, network: Network, password: String) -> Self {
        Self {
            db_pool,
            network,
            password,
        }
    }

    pub async fn create_unsigned_transaction(
        &self,
        account: &AccountRow,
        locked_funds: LockFundsResponse,
        recipients: Vec<Recipient>,
        fee_per_gram: MicroMinotari,
    ) -> Result<PrepareOneSidedTransactionForSigningResult, anyhow::Error> {
        if recipients.is_empty() {
            return Err(anyhow!("No recipients provided"));
        }
        if recipients.len() > 1 {
            return Err(anyhow!("Only one recipient is supported for now"));
        }

        let recipient = &recipients[0];

        let sender_address = account.get_address(self.network, &self.password)?;

        let key_manager = account.get_key_manager(&self.password).await?;
        let consensus_constants = ConsensusConstantsBuilder::new(self.network).build();
        let mut tx_builder = TransactionBuilder::new(consensus_constants, key_manager.clone(), self.network)?;

        tx_builder.with_fee_per_gram(fee_per_gram);

        for utxo in &locked_funds.utxos {
            tx_builder.with_input(utxo.clone())?;
        }

        let tx_id = TxId::new_random();
        let payment_id = match &recipient.payment_id {
            Some(s) => MemoField::new_open_from_string(s, TxType::PaymentToOther).map_err(|e| anyhow!(e))?,
            None => MemoField::new_empty(),
        };
        let output_features = OutputFeatures::default();
        let recipients = [PaymentRecipient {
            amount: recipient.amount,
            output_features: output_features.clone(),
            address: recipient.address.clone(),
            payment_id: payment_id.clone(),
        }];
        let result =
            prepare_one_sided_transaction_for_signing(tx_id, tx_builder, &recipients, payment_id, sender_address)?;

        Ok(result)
    }

    // pub async fn create_unsigned_transaction(
    //     &self,
    //     account: &AccountRow,
    //     locked_funds: LockFundsResponse,
    //     recipients: Vec<Recipient>,
    //     fee_per_gram: MicroMinotari,
    // ) -> Result<PrepareOneSidedTransactionForSigningResult, anyhow::Error> {
    //     if recipients.is_empty() {
    //         return Err(anyhow!("No recipients provided"));
    //     }
    //     if recipients.len() > 1 {
    //         return Err(anyhow!("Only one recipient is supported for now"));
    //     }

    //     if db::check_if_transaction_was_already_processed_by_idempotency_key(
    //         &mut conn,
    //         &signed_transaction.request.idempotency_key,
    //         account.id,
    //     )
    //     .await?
    //     {
    //         return Err(anyhow!(
    //             "A pending transaction with the same idempotency key already exists"
    //         ));
    //     }

    //     let recipient = recipients[0].clone();

    //     let sender_address = account.get_address(self.network, &self.password)?;

    //     let mut conn = self.db_pool.acquire().await?;
    //     let key_manager = account.get_key_manager(&self.password).await?;
    //     let consensus_constants = ConsensusConstantsBuilder::new(self.network).build();
    //     let mut tx_builder = TransactionBuilder::new(consensus_constants, key_manager.clone(), self.network)?;

    //     tx_builder.with_fee_per_gram(fee_per_gram);

    //     for utxo in &locked_funds.utxos {
    //         tx_builder.with_input(utxo.clone())?;
    //     }

    //     let tx_id = TxId::new_random();
    //     let tx_id_str = tx_id.to_string();

    //     let payment_id = match &recipient.payment_id {
    //         Some(s) => MemoField::new_open_from_string(s, TxType::PaymentToOther).map_err(|e| anyhow!(e))?,
    //         None => MemoField::new_empty(),
    //     };
    //     let output_features = OutputFeatures::default();

    //     let mut transaction = self.db_pool.begin().await?;

    //     let expires_at = Utc::now() + Duration::seconds(seconds_to_lock_utxos as i64);
    //     let unsigned_tx_json = serde_json::to_string(&result).map_err(|e| anyhow!(e))?;
    //     let idempotency_key = idempotency_key.unwrap_or_else(|| Uuid::new_v4().to_string());
    //     let pending_tx_id = db::create_pending_transaction(
    //         &mut transaction,
    //         &tx_id_str,
    //         &idempotency_key,
    //         account.id,
    //         &unsigned_tx_json,
    //         expires_at,
    //     )
    //     .await?;

    //     println!("Pending transaction {:?}", result);

    //     transaction.commit().await?;
    //     let recipients = [PaymentRecipient {
    //         amount: recipient.amount,
    //         output_features: output_features.clone(),
    //         address: recipient.address.clone(),
    //         payment_id: payment_id.clone(),
    //     }];

    //     let result = tokio::task::spawn_blocking(move || {
    //         prepare_one_sided_transaction_for_signing(tx_id, tx_builder, &recipients, payment_id, sender_address)
    //     })
    //     .await??;

    //     Ok(result)
    // }

    // pub async fn mark_transaction_as_signed(
    //     &self,
    //     account: AccountRow,
    //     signed_transaction: SignedOneSidedTransactionResult,
    //     grpc_address: String,
    // ) -> Result<(), anyhow::Error> {
    //     let mut conn = self.db_pool.acquire().await?;

    //     let tx_id = signed_transaction.request.tx_id.to_string();

    //     println!("Signed transaction: {:?}", signed_transaction);

    //     let now = Utc::now();
    //     let expires_at = chrono::DateTime::<Utc>::from_naive_utc_and_offset(pending_tx.expires_at, Utc);
    //     if expires_at < now {
    //         return Err(anyhow!("Pending transaction has expired"));
    //     }

    //     let kernel_excess = signed_transaction
    //         .signed_transaction
    //         .transaction
    //         .body()
    //         .kernels()
    //         .first()
    //         .map(|k| k.excess.to_vec());

    //     let sent_payref = signed_transaction
    //         .signed_transaction
    //         .sent_hashes
    //         .first()
    //         .map(|hash| hash.to_hex());

    //     let mut transaction = self.db_pool.begin().await?;

    //     db::update_pending_transaction_status(
    //         &mut transaction,
    //         &pending_tx.id,
    //         crate::models::PendingTransactionStatus::Completed,
    //     )
    //     .await?;

    //     let _completed_tx_id = db::create_completed_transaction(
    //         &mut transaction,
    //         &pending_tx.id,
    //         db::CompletedTransactionStatus::Completed,
    //         kernel_excess,
    //         sent_payref,
    //     )
    //     .await?;

    //     transaction.commit().await?;

    //     let wallet_http_client = WalletHttpClient::new(grpc_address.parse()?)?;

    //     let response = wallet_http_client
    //         .submit_transaction(signed_transaction.signed_transaction.transaction)
    //         .await?;

    //     println!("Transaction submitted successfully: {:?}", response);

    //     Ok(())
    // }
}
