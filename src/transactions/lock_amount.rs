use chrono::{Duration, Utc};
use sqlx::SqlitePool;
use tari_transaction_components::tari_amount::MicroMinotari;
use uuid::Uuid;

use crate::{
    api::types::LockFundsResponse,
    db::{self, AccountRow, ParentAccountRow},
    transactions::input_selector::InputSelector,
};

pub struct LockAmount {
    db_pool: SqlitePool,
}

impl LockAmount {
    pub fn new(db_pool: SqlitePool) -> Self {
        Self { db_pool }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn lock(
        &self,
        account: &ParentAccountRow,
        amount: MicroMinotari,
        num_outputs: usize,
        fee_per_gram: MicroMinotari,
        estimated_output_size: Option<usize>,
        idempotency_key: Option<String>,
        seconds_to_lock_utxos: u64,
    ) -> Result<LockFundsResponse, anyhow::Error> {
        let mut conn = self.db_pool.acquire().await?;
        if let Some(idempotency_key_str) = &idempotency_key
            && let Some(response) =
                db::find_pending_transaction_by_idempotency_key(&mut conn, idempotency_key_str, account.id).await?
        {
            return Ok(response);
        }

        let input_selector = InputSelector::new(account.id);
        let utxo_selection = input_selector
            .fetch_unspent_outputs(&mut conn, amount, num_outputs, fee_per_gram, estimated_output_size)
            .await?;

        let mut transaction = self.db_pool.begin().await?;

        let expires_at = Utc::now() + Duration::seconds(seconds_to_lock_utxos as i64);
        let idempotency_key = idempotency_key.unwrap_or_else(|| Uuid::new_v4().to_string());
        let pending_tx_id = db::create_pending_transaction(
            &mut transaction,
            &idempotency_key,
            account.id,
            utxo_selection.requires_change_output,
            utxo_selection.total_value,
            utxo_selection.fee_without_change,
            utxo_selection.fee_with_change,
            expires_at,
        )
        .await?;

        for utxo in &utxo_selection.utxos {
            db::lock_output(&mut transaction, utxo.id, &pending_tx_id, expires_at).await?;
        }

        transaction.commit().await?;

        Ok(LockFundsResponse {
            utxos: utxo_selection.utxos.iter().map(|utxo| utxo.output.clone()).collect(),
            requires_change_output: utxo_selection.requires_change_output,
            total_value: utxo_selection.total_value,
            fee_without_change: utxo_selection.fee_without_change,
            fee_with_change: utxo_selection.fee_with_change,
        })
    }
}
