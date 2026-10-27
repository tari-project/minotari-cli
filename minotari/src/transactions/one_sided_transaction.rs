//! One-sided (non-interactive) transaction construction.
//!
//! This module provides functionality for creating one-sided transactions, which are
//! transactions that can be sent without requiring any interaction from the recipient.
//! Unlike interactive transactions, one-sided transactions use the recipient's public
//! address to derive the necessary cryptographic components.
//!
//! # One-Sided Transactions
//!
//! One-sided transactions are the primary transaction type for Minotari. They allow
//! a sender to create a complete transaction using only the recipient's Tari address,
//! without any back-and-forth communication. The recipient can later claim the funds
//! by scanning the blockchain.
//!
//! # Transaction Flow
//!
//! 1. Lock UTXOs using [`FundLocker`](super::fund_locker::FundLocker)
//! 2. Create an unsigned transaction using [`OneSidedTransaction::create_unsigned_transaction`]
//! 3. Sign the transaction externally
//! 4. Broadcast the signed transaction to the network
//!
//! # Example
//!
//! ```rust,ignore
//! use minotari::transactions::one_sided_transaction::{OneSidedTransaction, Recipient};
//!
//! let tx_builder = OneSidedTransaction::new(db_pool, Network::MainNet, password);
//!
//! let recipient = Recipient {
//!     address: recipient_address,
//!     amount: MicroMinotari(1_000_000),
//!     payment_id: Some("Invoice #123".to_string()),
//! };
//!
//! let unsigned_tx = tx_builder.create_unsigned_transaction(
//!     &account,
//!     locked_funds,
//!     vec![recipient],
//!     MicroMinotari(5),
//! ).await?;
//! ```

use crate::db::SqlitePool;
use crate::transactions::fee_estimator::one_sided_output_sizes;
use crate::transactions::fund_locker::validate_fee_per_gram;
use crate::transactions::idempotency::{IdempotencyBinding, IdempotencyOperation, RequestFingerprint};
use crate::transactions::input_selector::map_change_required_error;
use crate::{api::types::LockFundsResult, db::AccountRow};
use anyhow::anyhow;
use log::info;
use tari_common::configuration::Network;
use tari_common_types::{tari_address::TariAddress, transaction::TxId};
use tari_transaction_components::offline_signing::models::PaymentRecipient;
use tari_transaction_components::{
    TransactionBuilder,
    consensus::ConsensusConstantsBuilder,
    offline_signing::{models::PrepareOneSidedTransactionForSigningResult, prepare_one_sided_transaction_for_signing},
    tari_amount::MicroMinotari,
    transaction_components::{MAX_TRANSACTION_OUTPUTS, MemoField, OutputFeatures, memo_field::TxType},
};
use zeroize::Zeroizing;

/// Represents a recipient of a one-sided transaction.
///
/// Contains all the information needed to send funds to a recipient,
/// including their address, the amount to send, and an optional payment
/// identifier for reference purposes.
///
/// # Fields
///
/// * `address` - The recipient's Tari address
/// * `amount` - The amount to send in MicroMinotari
/// * `payment_id` - Optional payment identifier/memo (e.g., invoice number)
///
/// # Example
///
/// ```rust,ignore
/// use minotari::transactions::one_sided_transaction::Recipient;
///
/// let recipient = Recipient {
///     address: TariAddress::from_base58("...")?,
///     amount: MicroMinotari(500_000),
///     payment_id: Some("Order-12345".to_string()),
/// };
/// ```
#[derive(Debug, Clone, Default)]
pub struct Recipient {
    /// The recipient's Tari address.
    pub address: TariAddress,
    /// The amount to send in MicroMinotari.
    pub amount: MicroMinotari,
    /// Optional payment identifier or memo attached to the transaction.
    pub payment_id: Option<String>,
}

/// The memo a recipient's output carries: the payment id, with the sender's address on it.
///
/// A one-sided payment has no handshake, so the encrypted memo is the only place the recipient's
/// wallet can learn who paid it. Without the address every wallet shows the sender as the
/// all-zero default address. This matches what the console wallet writes: fee zero, sender
/// one-sided, and `PaymentToSelf` when paying our own address.
///
/// Only the recipient's memo gets the address. The change memo is built by the transaction
/// builder from the plain payment id.
pub fn recipient_memo(
    payment_id: MemoField,
    sender_address: &TariAddress,
    recipient_address: &TariAddress,
) -> Result<MemoField, anyhow::Error> {
    let tx_type = if recipient_address == sender_address {
        TxType::PaymentToSelf
    } else {
        TxType::PaymentToOther
    };
    payment_id
        .add_sender_address(sender_address.clone(), true, MicroMinotari::zero(), Some(tx_type))
        .map_err(|e| anyhow!("Payment id does not fit alongside the sender address: {e}"))
}

impl Recipient {
    /// Its payment id as a plain memo, the transaction-level memo.
    pub fn memo(&self) -> Result<MemoField, anyhow::Error> {
        match &self.payment_id {
            Some(s) => MemoField::new_open_from_string(s, TxType::PaymentToOther).map_err(|e| anyhow!(e)),
            None => Ok(MemoField::new_empty()),
        }
    }
}

/// Checks `recipients` for a send and returns the sum of their amounts.
///
/// Fails when the list is empty or longer than a transaction can carry
/// alongside its change output, any amount is zero, the sum overflows, or a
/// payment id does not fit in its recipient's memo or the change memo. The
/// same address may appear more than once; it receives one output per entry.
pub fn total_recipient_amount(recipients: &[Recipient]) -> Result<MicroMinotari, anyhow::Error> {
    if recipients.is_empty() {
        return Err(anyhow!("No recipients provided"));
    }
    // One output is kept for the change.
    let max_recipients = MAX_TRANSACTION_OUTPUTS.saturating_sub(1);
    if recipients.len() > max_recipients {
        return Err(anyhow!("Too many recipients: at most {max_recipients} are allowed"));
    }
    let mut total = MicroMinotari::zero();
    for recipient in recipients {
        if recipient.amount == MicroMinotari::zero() {
            return Err(anyhow!("Recipient amount must be greater than zero"));
        }
        total = total
            .checked_add(recipient.amount)
            .ok_or_else(|| anyhow!("Total amount of the recipients overflows"))?;
    }
    // Builds every memo the send will carry, so a payment id that does not fit fails here.
    one_sided_output_sizes(recipients)?;
    Ok(total)
}

/// Binds an idempotency key to an unsigned-transaction request.
///
/// The recipients are the whole point of the binding. Without them, replaying a
/// client's key with a different `recipients` list would return that client's
/// locked UTXOs and build an unsigned transaction paying whoever the replay
/// named. Every caller of `create_unsigned_transaction` must build its binding
/// here so no entry point can forget a field.
pub fn unsigned_transaction_binding(
    idempotency_key: Option<String>,
    account_id: i64,
    recipients: &[Recipient],
    fee_per_gram: MicroMinotari,
    seconds_to_lock_utxos: u64,
    confirmation_window: u64,
    send_all: bool,
) -> IdempotencyBinding {
    let operation = IdempotencyOperation::CreateUnsignedTransaction;
    let mut fingerprint = RequestFingerprint::new(operation)
        .field("account_id", account_id.to_le_bytes())
        .field("fee_per_gram", fee_per_gram.as_u64().to_le_bytes())
        .field("seconds_to_lock_utxos", seconds_to_lock_utxos.to_le_bytes())
        .field("confirmation_window", confirmation_window.to_le_bytes())
        // Bound the list itself, so dropping or appending a recipient cannot be
        // hidden by the remaining entries hashing the same.
        .field("recipient_count", (recipients.len() as u64).to_le_bytes());
    for recipient in recipients {
        fingerprint = fingerprint
            .field("recipient_address", recipient.address.to_base58())
            .field("recipient_amount", recipient.amount.as_u64().to_le_bytes())
            .optional_field("recipient_payment_id", recipient.payment_id.as_deref());
    }
    if send_all {
        fingerprint = fingerprint.field("send_all", [1u8]);
    }
    IdempotencyBinding::new(idempotency_key, operation, fingerprint)
}

/// Builder for creating unsigned one-sided transactions.
///
/// `OneSidedTransaction` prepares transactions that can be sent without recipient
/// interaction. It handles the construction of transaction inputs, outputs, and
/// metadata required for offline signing.
///
/// # Security
///
/// The password provided is used to decrypt the account's key manager for
/// transaction construction. Ensure passwords are handled securely and not
/// logged or persisted unnecessarily.
///
/// # Example
///
/// ```rust,ignore
/// use minotari::transactions::one_sided_transaction::OneSidedTransaction;
///
/// let builder = OneSidedTransaction::new(
///     db_pool,
///     Network::MainNet,
///     "secure_password".to_string(),
/// );
///
/// let unsigned = builder.create_unsigned_transaction(
///     &account,
///     locked_funds,
///     recipients,
///     fee_per_gram,
/// ).await?;
/// ```
pub struct OneSidedTransaction {
    /// Database connection pool.
    pub db_pool: SqlitePool,
    /// The network (MainNet, TestNet, etc.) for consensus rules.
    pub network: Network,
    /// Password for decrypting the account's key manager.
    pub password: Zeroizing<String>,
}

impl OneSidedTransaction {
    /// Creates a new `OneSidedTransaction` builder.
    ///
    /// # Arguments
    ///
    /// * `db_pool` - SQLite connection pool for database operations
    /// * `network` - The Tari network (affects consensus constants)
    /// * `password` - Password to decrypt the account's key manager
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let builder = OneSidedTransaction::new(db_pool, Network::MainNet, password);
    /// ```
    pub fn new(db_pool: SqlitePool, network: Network, password: Zeroizing<String>) -> Self {
        Self {
            db_pool,
            network,
            password,
        }
    }

    /// Creates an unsigned one-sided transaction ready for signing.
    ///
    /// Constructs a transaction using the locked UTXOs as inputs and creates
    /// outputs for the specified recipients. The resulting transaction is
    /// prepared for offline signing.
    ///
    /// # Arguments
    ///
    /// * `account` - The sender's account containing key material
    /// * `locked_funds` - Previously locked UTXOs from [`FundLocker::lock`](super::fund_locker::FundLocker::lock)
    /// * `recipients` - List of recipients, one output each
    /// * `fee_per_gram` - Fee rate in MicroMinotari per gram
    ///
    /// # Returns
    ///
    /// Returns a [`PrepareOneSidedTransactionForSigningResult`] containing:
    /// - The unsigned transaction ready for signing
    /// - Metadata required to complete the signing process
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - No recipients are provided
    /// - Account key manager cannot be decrypted
    /// - Transaction building fails
    /// - Payment ID encoding fails
    ///
    /// # Memos
    ///
    /// Each recipient's output carries that recipient's own `payment_id`. The
    /// transaction-level memo (kept in the change output) is the first
    /// recipient's `payment_id`.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let recipient = Recipient {
    ///     address: recipient_address,
    ///     amount: MicroMinotari(1_000_000),
    ///     payment_id: Some("Payment for services".to_string()),
    /// };
    ///
    /// let unsigned_tx = builder.create_unsigned_transaction(
    ///     &account,
    ///     locked_funds,
    ///     vec![recipient],
    ///     MicroMinotari(5),
    /// ).await?;
    ///
    /// // Sign the transaction externally
    /// // let signed = sign(unsigned_tx)?;
    /// ```
    pub fn create_unsigned_transaction(
        &self,
        account: &AccountRow,
        locked_funds: LockFundsResult,
        recipients: Vec<Recipient>,
        fee_per_gram: MicroMinotari,
    ) -> Result<PrepareOneSidedTransactionForSigningResult, anyhow::Error> {
        if recipients.is_empty() {
            return Err(anyhow!("No recipients provided"));
        }
        validate_fee_per_gram(fee_per_gram)?;

        info!(
            target: "audit",
            count = recipients.len();
            "Creating unsigned one-sided transaction"
        );

        let sender_address = account.get_address(self.network, &self.password)?;

        let key_manager = account.get_key_manager(&self.password)?;
        let consensus_constants = ConsensusConstantsBuilder::new(self.network).build();
        let mut tx_builder = TransactionBuilder::new(consensus_constants, key_manager.clone(), self.network)?;

        tx_builder.with_fee_per_gram(fee_per_gram);
        // Every send carries a change output, so the wallet's scan can see it land.
        tx_builder.with_change_required();

        for utxo in &locked_funds.utxos {
            tx_builder.with_input(utxo.clone())?;
        }

        let tx_id = TxId::new_random();
        let output_features = OutputFeatures::default();
        let mut payment_recipients = Vec::with_capacity(recipients.len());
        for r in &recipients {
            payment_recipients.push(PaymentRecipient {
                amount: r.amount,
                output_features: output_features.clone(),
                address: r.address.clone(),
                payment_id: recipient_memo(r.memo()?, &sender_address, &r.address)?,
            });
        }

        // Use first recipient's payment_id, without the sender address, as the overall transaction memo
        let main_payment_id = recipients.first().expect("Already checked").memo()?;

        let result = prepare_one_sided_transaction_for_signing(
            &key_manager,
            tx_id,
            tx_builder,
            &payment_recipients,
            main_payment_id,
            sender_address,
        )
        .map_err(map_change_required_error)?;

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recipient(payment_id: Option<String>) -> Recipient {
        Recipient {
            address: TariAddress::default(),
            amount: MicroMinotari(1_000),
            payment_id,
        }
    }

    #[test]
    fn a_send_may_have_one_recipient_fewer_than_a_transaction_has_outputs() {
        let most = vec![recipient(None); MAX_TRANSACTION_OUTPUTS - 1];
        assert_eq!(
            total_recipient_amount(&most).expect("allowed"),
            MicroMinotari(1_000 * (MAX_TRANSACTION_OUTPUTS as u64 - 1))
        );
        let too_many = vec![recipient(None); MAX_TRANSACTION_OUTPUTS];
        let err = total_recipient_amount(&too_many).expect_err("no room for change");
        assert!(err.to_string().contains("Too many recipients"), "got: {err}");
    }

    /// A payment id that fits neither memo is refused, not sent without its memo.
    #[test]
    fn a_payment_id_too_long_for_a_memo_is_refused() {
        total_recipient_amount(&[recipient(Some("invoice-1".to_string()))]).expect("fits");
        total_recipient_amount(&[recipient(Some("x".repeat(300)))]).expect_err("too long for any memo");
        // Fits a recipient's open memo, but not the change memo, which also holds an address and a hash.
        let long = "x".repeat(200);
        assert!(MemoField::new_open_from_string(&long, TxType::PaymentToOther).is_ok());
        total_recipient_amount(&[recipient(Some(long))]).expect_err("too long for the change memo");
    }
}
