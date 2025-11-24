use anyhow::anyhow;
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

use crate::{ParentAccountRow, api::types::LockFundsResponse, db::AccountRow};

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
        account: &ParentAccountRow,
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

        Ok(result)
    }
}
