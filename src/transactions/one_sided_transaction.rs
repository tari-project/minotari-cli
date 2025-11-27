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
}
