use sqlx::SqliteConnection;
use tari_script::TariScript;
use tari_transaction_components::{
    fee::Fee,
    helpers::borsh::SerializedSize,
    tari_amount::MicroMinotari,
    transaction_components::{OutputFeatures, covenants::Covenant},
    weight::TransactionWeight,
};
use thiserror::Error;

use crate::db::{DbWalletOutput, get_latest_scanned_tip_block_by_account};

#[derive(Debug, Error)]
pub enum UtxoSelectionError {
    #[error("Serialization error: {0}")]
    SerializationError(String),
    #[error("Not enough funds. Available: {available}, required: {required}")]
    InsufficientFunds {
        available: MicroMinotari,
        required: MicroMinotari,
    },
    #[error("DB error")]
    DbError(#[from] sqlx::Error),
}

#[derive(Debug)]
pub struct UtxoSelection {
    pub utxos: Vec<DbWalletOutput>,
    pub requires_change_output: bool,
    pub total_value: MicroMinotari,
    pub fee_without_change: MicroMinotari,
    pub fee_with_change: MicroMinotari,
}

impl UtxoSelection {
    pub fn fee(&self) -> MicroMinotari {
        if self.requires_change_output {
            self.fee_with_change
        } else {
            self.fee_without_change
        }
    }
}

pub struct InputSelector {
    account_id: i64,
    confirmation_window: u64,
    fee_calc: Fee,
}

impl InputSelector {
    pub fn new(account_id: i64, confirmation_window: u64) -> Self {
        Self {
            account_id,
            confirmation_window,
            fee_calc: Fee::new(TransactionWeight::latest()),
        }
    }

    fn get_features_and_scripts_byte_size(&self) -> Result<usize, UtxoSelectionError> {
        let output_features_size = OutputFeatures::default()
            .get_serialized_size()
            .map_err(|e| UtxoSelectionError::SerializationError(e.to_string()))?;
        let tari_script_size = TariScript::default()
            .get_serialized_size()
            .map_err(|e| UtxoSelectionError::SerializationError(e.to_string()))?;
        let covenant_size = Covenant::default()
            .get_serialized_size()
            .map_err(|e| UtxoSelectionError::SerializationError(e.to_string()))?;

        Ok(self
            .fee_calc
            .weighting()
            .round_up_features_and_scripts_size(output_features_size + tari_script_size + covenant_size))
    }

    pub async fn fetch_unspent_outputs(
        &self,
        conn: &mut SqliteConnection,
        amount: MicroMinotari,
        num_outputs: usize,
        fee_per_gram: MicroMinotari,
        estimated_output_size: Option<usize>,
    ) -> Result<UtxoSelection, UtxoSelectionError> {
        let tip = get_latest_scanned_tip_block_by_account(conn, self.account_id).await?;
        let min_height = tip
            .map(|b| b.height)
            .unwrap_or(0)
            .saturating_sub(self.confirmation_window);

        let uo = crate::db::fetch_unspent_outputs(&mut *conn, self.account_id, min_height).await?;

        let features_and_scripts_byte_size = match estimated_output_size {
            Some(sz) => sz,
            None => self.get_features_and_scripts_byte_size()?,
        };

        let mut sufficient_funds = false;
        let mut utxos = Vec::new();
        let mut requires_change_output = false;
        let mut total_value = MicroMinotari::zero();
        let mut fee_without_change = MicroMinotari::zero();
        let mut fee_with_change = MicroMinotari::zero();

        for o in uo {
            total_value += o.output.value();
            utxos.push(o);

            fee_without_change = self.fee_calc.calculate(
                fee_per_gram,
                1,
                utxos.len(),
                num_outputs,
                features_and_scripts_byte_size,
            );
            if total_value == amount + fee_without_change {
                sufficient_funds = true;
                break;
            }
            fee_with_change = self.fee_calc.calculate(
                fee_per_gram,
                1,
                utxos.len(),
                num_outputs + 1,
                2 * features_and_scripts_byte_size,
            );

            if total_value > amount + fee_with_change {
                sufficient_funds = true;
                requires_change_output = true;
                break;
            }
        }

        if !sufficient_funds {
            return Err(UtxoSelectionError::InsufficientFunds {
                available: total_value,
                required: amount + fee_with_change,
            });
        }

        Ok(UtxoSelection {
            utxos,
            requires_change_output,
            total_value,
            fee_without_change,
            fee_with_change,
        })
    }
}
