//! UTXO selection algorithms for transaction construction.
//!
//! This module implements the input selection logic used when constructing
//! transactions. It handles the complex task of selecting appropriate UTXOs
//! to cover the desired transaction amount plus fees, while minimizing waste
//! and determining whether a change output is needed.
//!
//! # Selection Strategy
//!
//! The current implementation uses a simple accumulator strategy:
//!
//! 1. Fetch all unspent outputs for the account, ordered by value
//! 2. Accumulate outputs until the total covers amount + fees
//! 3. Calculate whether a change output is needed based on exact matching
//!
//! # Fee Calculation
//!
//! Fees are calculated based on:
//! - Transaction weight (number of inputs and outputs)
//! - Output features and scripts size
//! - Current fee per gram rate
//!
//! The selector calculates fees both with and without a change output,
//! allowing the caller to choose the optimal strategy.
//!
//! # Example
//!
//! ```rust,ignore
//! use minotari::transactions::input_selector::InputSelector;
//!
//! let selector = InputSelector::new(account_id);
//!
//! let selection = selector.fetch_unspent_outputs(
//!     &mut conn,
//!     MicroMinotari(1_000_000), // amount to send
//!     1,                        // number of outputs
//!     MicroMinotari(5),         // fee per gram
//!     None,                     // use default output size
//! ).await?;
//!
//! println!("Selected {} UTXOs, total: {}", selection.utxos.len(), selection.total_value);
//! println!("Fee: {}", selection.fee());
//! ```

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

/// Errors that can occur during UTXO selection.
#[derive(Debug, Error)]
pub enum UtxoSelectionError {
    /// Failed to serialize transaction components for size calculation.
    #[error("Serialization error: {0}")]
    SerializationError(String),

    /// The account does not have enough funds to cover the requested amount plus fees.
    ///
    /// This error includes both the available balance and the required amount
    /// to help users understand the shortfall.
    #[error("Not enough funds. Available: {available}, required: {required}")]
    InsufficientFunds {
        /// The total value of all available UTXOs.
        available: MicroMinotari,
        /// The amount needed (transaction amount + estimated fees).
        required: MicroMinotari,
    },

    /// A database operation failed.
    #[error("DB error")]
    DbError(#[from] sqlx::Error),
}

/// Result of UTXO selection for a transaction.
///
/// Contains the selected UTXOs along with fee calculations and metadata
/// about whether a change output is required.
///
/// # Fee Selection
///
/// The [`fee`](Self::fee) method returns the appropriate fee based on
/// whether a change output is needed. Use `fee_without_change` when the
/// selected UTXOs exactly match the required amount, or `fee_with_change`
/// when excess value must be returned as change.
#[derive(Debug)]
pub struct UtxoSelection {
    /// The selected unspent transaction outputs.
    pub utxos: Vec<DbWalletOutput>,

    /// Whether the selection requires a change output.
    ///
    /// `true` if the total value exceeds amount + fee, requiring change.
    /// `false` if the total exactly matches amount + fee.
    pub requires_change_output: bool,

    /// Total value of all selected UTXOs.
    pub total_value: MicroMinotari,

    /// Calculated fee assuming no change output is needed.
    pub fee_without_change: MicroMinotari,

    /// Calculated fee including a change output.
    pub fee_with_change: MicroMinotari,
}

impl UtxoSelection {
    /// Returns the appropriate fee based on whether change is required.
    ///
    /// If [`requires_change_output`](Self::requires_change_output) is `true`,
    /// returns [`fee_with_change`](Self::fee_with_change); otherwise returns
    /// [`fee_without_change`](Self::fee_without_change).
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let selection = selector.fetch_unspent_outputs(...).await?;
    /// let actual_fee = selection.fee();
    /// let change_amount = selection.total_value - amount - actual_fee;
    /// ```
    pub fn fee(&self) -> MicroMinotari {
        if self.requires_change_output {
            self.fee_with_change
        } else {
            self.fee_without_change
        }
    }
}

/// Selects UTXOs for transaction inputs with fee calculation.
///
/// `InputSelector` implements the UTXO selection algorithm used when
/// constructing transactions. It queries the database for available
/// outputs and selects a minimal set sufficient to cover the desired
/// amount plus transaction fees.
///
/// # Algorithm
///
/// The selector uses a greedy accumulation strategy:
/// 1. Query all unspent outputs for the account
/// 2. Iterate through outputs, accumulating value
/// 3. For each accumulated total, calculate fees with and without change
/// 4. Stop when the total covers amount + fees
///
/// # Example
///
/// ```rust,ignore
/// use minotari::transactions::input_selector::InputSelector;
///
/// let selector = InputSelector::new(account_id);
///
/// let selection = selector.fetch_unspent_outputs(
///     &mut conn,
///     MicroMinotari(500_000),
///     1,
///     MicroMinotari(5),
///     None,
/// ).await?;
///
/// if selection.requires_change_output {
///     println!("Change required: {}", selection.total_value - amount - selection.fee());
/// }
/// ```
pub struct InputSelector {
    account_id: i64,
    confirmation_window: u64,
    fee_calc: Fee,
}

impl InputSelector {
    /// Creates a new `InputSelector` for the specified account.
    ///
    /// Initializes the selector with the latest transaction weight rules
    /// for fee calculation.
    ///
    /// # Arguments
    ///
    /// * `account_id` - The account ID whose UTXOs will be selected
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let selector = InputSelector::new(account_id);
    /// ```
    pub fn new(account_id: i64, confirmation_window: u64) -> Self {
        Self {
            account_id,
            confirmation_window,
            fee_calc: Fee::new(TransactionWeight::latest()),
        }
    }

    /// Calculates the byte size of default output features and scripts.
    ///
    /// This is used for fee estimation when no custom output size is provided.
    /// The calculation includes:
    /// - Default output features
    /// - Default Tari script
    /// - Default covenant
    ///
    /// # Returns
    ///
    /// The rounded-up byte size for fee calculation purposes.
    ///
    /// # Errors
    ///
    /// Returns [`UtxoSelectionError::SerializationError`] if any component
    /// fails to serialize.
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

    /// Fetches and selects UTXOs sufficient to cover the requested amount plus fees.
    ///
    /// This method queries the database for unspent outputs and accumulates them
    /// until the total value covers the requested amount plus calculated transaction
    /// fees. It determines whether a change output is needed and calculates fees
    /// for both scenarios.
    ///
    /// # Arguments
    ///
    /// * `conn` - Database connection for querying UTXOs
    /// * `amount` - The amount to be sent (excluding fees)
    /// * `num_outputs` - Number of transaction outputs (recipient outputs, excluding change)
    /// * `fee_per_gram` - Fee rate in MicroMinotari per gram of transaction weight
    /// * `estimated_output_size` - Optional custom output size for fee calculation;
    ///   if `None`, uses default calculation from [`get_features_and_scripts_byte_size`](Self::get_features_and_scripts_byte_size)
    ///
    /// # Returns
    ///
    /// Returns a [`UtxoSelection`] containing:
    /// - The selected UTXOs
    /// - Whether a change output is required
    /// - Total value of selected UTXOs
    /// - Fee calculations with and without change
    ///
    /// # Errors
    ///
    /// Returns [`UtxoSelectionError::InsufficientFunds`] if the account balance
    /// cannot cover the requested amount plus fees.
    ///
    /// Returns [`UtxoSelectionError::DbError`] if the database query fails.
    ///
    /// Returns [`UtxoSelectionError::SerializationError`] if output size
    /// calculation fails (when `estimated_output_size` is `None`).
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let selector = InputSelector::new(account_id);
    ///
    /// let selection = selector.fetch_unspent_outputs(
    ///     &mut conn,
    ///     MicroMinotari(1_000_000),
    ///     1,  // one recipient
    ///     MicroMinotari(5),
    ///     None,
    /// ).await?;
    ///
    /// // Calculate change amount if needed
    /// if selection.requires_change_output {
    ///     let change = selection.total_value - amount - selection.fee_with_change;
    ///     println!("Change: {}", change);
    /// }
    /// ```
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
