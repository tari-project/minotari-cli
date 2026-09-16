use anyhow::{Result, anyhow};
use log::debug;
use tari_common_types::{tari_address::TariAddress, types::FixedHash};
use tari_script::{TariScript, script};
use tari_transaction_components::{
    fee::{Fee, recipient_output_features_and_scripts_size},
    tari_amount::MicroMinotari,
    transaction_components::{
        OutputFeatures,
        covenants::Covenant,
        memo_field::{MemoField, TxType},
    },
    weight::TransactionWeight,
};

use crate::http::WalletHttpClient;
use crate::{
    db::{self, AccountRow, SqlitePool},
    transactions::input_selector::InputSelector,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeePriority {
    Slow,
    Medium,
    Fast,
}

#[derive(Debug, Clone)]
pub struct FeeEstimateResult {
    pub priority: FeePriority,
    pub fee_per_gram: MicroMinotari,
    pub estimated_fee: MicroMinotari,
    pub total_amount_required: MicroMinotari,
    pub input_count: usize,
}

pub struct FeeEstimator {
    db_pool: SqlitePool,
    base_url: String,
    fee_calc: Fee,
}

impl FeeEstimator {
    pub fn new(db_pool: SqlitePool, base_url: String) -> Self {
        Self {
            db_pool,
            base_url,
            fee_calc: Fee::new(TransactionWeight::latest()),
        }
    }

    pub async fn estimate_fees(
        &self,
        account_name: &str,
        amount: MicroMinotari,
        num_outputs: usize,
        confirmation_window: u64,
        estimated_output_size: Option<usize>,
    ) -> Result<Vec<FeeEstimateResult>> {
        let conn = self.db_pool.get()?;

        let account: AccountRow = db::get_account_by_name(&conn, account_name)?
            .ok_or_else(|| anyhow!("Account with name '{}' not found", account_name))?;

        let client = WalletHttpClient::new(self.base_url.parse()?)?;
        let (fast_fee, medium_fee, slow_fee) = match client.get_mempool_fee_per_gram_stats(3).await {
            Ok(stats) if !stats.is_empty() => {
                // Fast: Average of the 1st block (next block)
                let fast = stats.first().expect("Already checked").avg_fee_per_gram;
                // Medium: Average of the 2nd block (if exists)
                let medium = stats
                    .get(1)
                    .unwrap_or(stats.first().expect("Already checked"))
                    .avg_fee_per_gram;
                // Slow: Minimum of the deepest block we got
                let slow = stats.last().expect("Already checked").min_fee_per_gram;

                (fast, medium, slow)
            },
            _ => (MicroMinotari::from(10), MicroMinotari::from(5), MicroMinotari::from(1)),
        };

        let input_selector = InputSelector::new(account.id, confirmation_window);

        let selection =
            input_selector.fetch_unspent_outputs(&conn, amount, num_outputs, fast_fee, estimated_output_size)?;

        let input_count = selection.utxos.len();
        let total_outputs = if selection.requires_change_output {
            num_outputs + 1
        } else {
            num_outputs
        };

        let output_size = match estimated_output_size {
            Some(sz) => sz,
            None => get_default_features_and_scripts_size()?,
        };

        let results = [
            (FeePriority::Slow, slow_fee),
            (FeePriority::Medium, medium_fee),
            (FeePriority::Fast, fast_fee),
        ]
        .into_iter()
        .map(|(priority, fee_per_gram)| {
            self.calculate_single_estimate(priority, fee_per_gram, amount, input_count, total_outputs, output_size)
        })
        .collect();

        debug!(
            account = account_name,
            inputs = input_count;
            "Calculated fee estimates"
        );

        Ok(results)
    }

    fn calculate_single_estimate(
        &self,
        priority: FeePriority,
        fee_per_gram: MicroMinotari,
        amount: MicroMinotari,
        input_count: usize,
        output_count: usize,
        output_size: usize,
    ) -> FeeEstimateResult {
        let fee = self
            .fee_calc
            .calculate(fee_per_gram, 1, input_count, output_count, output_size * output_count);

        FeeEstimateResult {
            priority,
            fee_per_gram,
            estimated_fee: fee,
            total_amount_required: amount + fee,
            input_count,
        }
    }
}

/// Measure one output the way the transaction builder charges for it.
///
/// Upstream's `recipient_output_features_and_scripts_size` is the authority here: it counts
/// the features, the script, the covenant **and the memo carried in the output's encrypted
/// data**, then rounds up to the fee's gram boundary. Summing the first three by hand - which
/// is what this module used to do - silently drops the memo, which is usually the largest of
/// the four.
pub fn measure_output_size(features: &OutputFeatures, script: &TariScript, memo: &MemoField) -> Result<usize> {
    recipient_output_features_and_scripts_size(
        &TransactionWeight::latest(),
        features,
        script,
        &Covenant::default(),
        memo,
    )
    .map_err(|e| anyhow!("Failed to measure output size: {e}"))
}

/// A conservative per-output size for a payment whose memo carries `payment_id_len` bytes.
///
/// UTXO selection charges this once per output and once for change, so it must over-estimate
/// rather than under-estimate: a short answer locks inputs that cannot cover the fee the
/// builder later computes, and `reserve_sender_offset_keys` then fails the send *after* the
/// funds are reserved, leaving the user to wait out the lock. Over-estimating only makes the
/// change output slightly smaller than it needed to be.
///
/// The shape measured is the change output, which is the largest the builder emits on its own
/// account: a `PushPubKey` script and a `TransactionInfo` memo holding the recipient address,
/// one sent-output hash and the caller's payment id, padded to a 130-byte floor.
pub fn estimated_output_size_for_payment_id(payment_id_len: usize) -> Result<usize> {
    // The script the builder derives for a recipient, and for its own change output.
    let script = script!(PushPubKey(Box::default())).map_err(|e| anyhow!("Failed to build the default script: {e}"))?;

    // A change memo: a dual address, one sent-output hash and the payment id. `TariAddress`
    // defaults to the dual form, which is the larger of the two it can take.
    let change_memo = |payment_id: Vec<u8>| {
        MemoField::new_transaction_info(
            TariAddress::default(),
            MicroMinotari::zero(),
            MicroMinotari::zero(),
            true,
            TxType::PaymentToOther,
            vec![FixedHash::zero()],
            payment_id,
        )
    };

    // A memo has a hard 256-byte ceiling, so a payment id past it cannot go in a change memo
    // at all - upstream's own `change_features_and_scripts_size` measures such a memo as zero
    // and the send fails when the builder tries to construct it for real. There is no estimate
    // that saves that transaction, so fall back to the floor rather than refusing to quote.
    let memo = change_memo(vec![0u8; payment_id_len])
        .or_else(|_| change_memo(Vec::new()))
        .map_err(|e| anyhow!("Failed to build the default change memo: {e}"))?;

    measure_output_size(&OutputFeatures::default(), &script, &memo)
}

/// The per-output size to assume when even the payment id is unknown.
pub fn get_default_features_and_scripts_size() -> Result<usize> {
    estimated_output_size_for_payment_id(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tari_script::script;

    /// The estimate has to cover what the builder actually charges, or selection locks inputs
    /// that cannot pay the fee and the send dies after the funds are reserved.
    #[test]
    fn the_default_estimate_covers_a_one_sided_recipient_output() {
        let recipient_memo = MemoField::new_open_from_string("invoice-12345", TxType::PaymentToOther).unwrap();
        let recipient = measure_output_size(
            &OutputFeatures::default(),
            &script!(PushPubKey(Box::default())).unwrap(),
            &recipient_memo,
        )
        .unwrap();

        assert!(
            get_default_features_and_scripts_size().unwrap() >= recipient,
            "default estimate must not be smaller than a real recipient output"
        );
    }

    /// The old implementation summed features + an *empty* script + covenant and stopped, which
    /// is where the under-estimate came from. Both missing terms have to be back.
    #[test]
    fn the_default_estimate_counts_the_script_and_the_memo() {
        let bare = measure_output_size(
            &OutputFeatures::default(),
            &TariScript::default(),
            &MemoField::new_empty(),
        )
        .unwrap();

        assert!(
            get_default_features_and_scripts_size().unwrap() > bare,
            "default estimate must count the script and the memo, not just features and covenant"
        );
    }

    /// A longer payment id is a bigger output, and selection has to be told so.
    #[test]
    fn a_longer_payment_id_raises_the_estimate() {
        let short = estimated_output_size_for_payment_id(0).unwrap();
        let long = estimated_output_size_for_payment_id(100).unwrap();
        assert!(long > short, "a 100-byte payment id must cost more than an empty one");
    }
    /// A payment id too long for a memo cannot be quoted exactly; the estimate falls back to
    /// the floor instead of failing the caller's request outright.
    #[test]
    fn an_oversized_payment_id_falls_back_to_the_floor() {
        assert_eq!(
            estimated_output_size_for_payment_id(4096).unwrap(),
            estimated_output_size_for_payment_id(0).unwrap()
        );
    }
}
