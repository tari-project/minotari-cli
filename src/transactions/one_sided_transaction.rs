use anyhow::anyhow;
use chrono::{Duration, Utc};
use sqlx::SqlitePool;
use tari_common::configuration::Network;
use tari_common_types::{tari_address::TariAddress, transaction::TxId};
use tari_transaction_components::{
    TransactionBuilder,
    consensus::ConsensusConstantsBuilder,
    offline_signing::{models::PrepareOneSidedTransactionForSigningResult, offline_signer::OfflineSigner},
    tari_amount::MicroMinotari,
    transaction_components::{MemoField, OutputFeatures, memo_field::TxType},
};
use uuid::Uuid;

use crate::{
    db::{self, AccountRow},
    transactions::input_selector::InputSelector,
};

#[derive(Debug, Clone)]
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
        account: AccountRow,
        recipients: Vec<Recipient>,
        idempotency_key: Option<String>,
        seconds_to_lock_utxos: u64,
    ) -> Result<PrepareOneSidedTransactionForSigningResult, anyhow::Error> {
        let mut conn = self.db_pool.acquire().await?;
        if let Some(idempotency_key_str) = &idempotency_key
            && let Some(unsigned_tx_json) =
                db::find_pending_transaction_by_idempotency_key(&mut conn, idempotency_key_str, account.id).await?
        {
            let result: PrepareOneSidedTransactionForSigningResult =
                serde_json::from_str(&unsigned_tx_json).map_err(|e| anyhow!(e))?;
            return Ok(result);
        }

        if recipients.is_empty() {
            return Err(anyhow!("No recipients provided"));
        }
        if recipients.len() > 1 {
            return Err(anyhow!("Only one recipient is supported for now"));
        }

        let recipient = &recipients[0];

        let sender_address = account.get_address(self.network, &self.password)?;

        let input_selector = InputSelector::new(account.id);
        let fee_per_gram = MicroMinotari(5);
        let utxo_selection = input_selector
            .fetch_unspent_outputs(&mut conn, recipient.amount, fee_per_gram)
            .await?;

        let key_manager = account.get_key_manager(&self.password).await?;
        let consensus_constants = ConsensusConstantsBuilder::new(self.network).build();
        let mut tx_builder = TransactionBuilder::new(consensus_constants, key_manager.clone(), self.network)?;

        tx_builder.with_fee_per_gram(fee_per_gram);

        for utxo in &utxo_selection.utxos {
            tx_builder.with_input(utxo.output.clone())?;
        }

        let mut offline_signing = OfflineSigner::new(key_manager);
        let tx_id = TxId::new_random();
        let payment_id = match &recipient.payment_id {
            Some(s) => MemoField::new_open_from_string(s, TxType::PaymentToOther).map_err(|e| anyhow!(e))?,
            None => MemoField::new_empty(),
        };
        let output_features = OutputFeatures::default();

        let result = offline_signing.prepare_one_sided_transaction_for_signing(
            tx_id,
            tx_builder,
            recipient.address.clone(),
            recipient.amount,
            output_features,
            payment_id,
            sender_address,
        )?;

        let mut transaction = self.db_pool.begin().await?;

        let expires_at = Utc::now() + Duration::seconds(seconds_to_lock_utxos as i64);
        let unsigned_tx_json = serde_json::to_string(&result).map_err(|e| anyhow!(e))?;
        let idempotency_key = idempotency_key.unwrap_or_else(|| Uuid::new_v4().to_string());
        let pending_tx_id = db::create_pending_transaction(
            &mut transaction,
            &idempotency_key,
            account.id,
            &unsigned_tx_json,
            expires_at,
        )
        .await?;

        for utxo in utxo_selection.utxos {
            db::lock_output(&mut transaction, utxo.id, &pending_tx_id, expires_at).await?;
        }

        transaction.commit().await?;

        Ok(result)
    }
}
