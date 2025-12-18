//! Account management API endpoint handlers.
//!
//! This module provides HTTP endpoint handlers for account-related operations
//! in the Minotari wallet REST API. It includes functionality for:
//!
//! - Querying account balances
//! - Locking funds for transaction preparation
//! - Creating unsigned transactions for one-sided payments
//!
//! All endpoints follow RESTful conventions and return JSON responses.
//! Error responses use the [`ApiError`] type for consistent error formatting.
//!
//! # Endpoint Overview
//!
//! | Method | Path | Description |
//! |--------|------|-------------|
//! | GET | `/accounts/{name}/balance` | Retrieve account balance |
//! | POST | `/accounts/{name}/lock_funds` | Lock UTXOs for spending |
//! | POST | `/accounts/{name}/create_unsigned_transaction` | Create unsigned transaction |
//!
//! # Example Usage
//!
//! ```bash
//! # Get account balance
//! curl -X GET http://localhost:8080/accounts/default/balance
//!
//! # Lock funds for a transaction
//! curl -X POST http://localhost:8080/accounts/default/lock_funds \
//!   -H "Content-Type: application/json" \
//!   -d '{"amount": 1000000}'
//! ```

use axum::{
    Json,
    extract::{Path, State},
};
use serde::Deserialize;
use serde_json::{Value as JsonValue, json};
use utoipa::{
    IntoParams,
    openapi::{ObjectBuilder, Schema, Type, schema::SchemaType},
};

use super::error::ApiError;
use crate::{
    api::{
        AppState,
        types::{LockFundsResult, TariAddressBase58},
    },
    db::{AccountBalance, get_account_by_name, get_balance},
    transactions::{
        fund_locker::FundLocker,
        monitor::REQUIRED_CONFIRMATIONS,
        one_sided_transaction::{OneSidedTransaction, Recipient},
    },
};
use tari_transaction_components::tari_amount::MicroMinotari;

/// Returns the default lock duration for UTXOs.
///
/// UTXOs are locked for 24 hours (86,400 seconds) by default to prevent
/// double-spending while a transaction is being prepared and broadcast.
fn default_seconds_to_lock_utxos() -> Option<u64> {
    Some(86400)
}

/// Returns the default number of outputs for a transaction.
///
/// Defaults to 1 output, which is suitable for simple single-recipient
/// transactions.
fn default_num_outputs() -> Option<usize> {
    Some(1)
}

/// Returns the default fee per gram for transactions.
///
/// The default fee is 5 MicroMinotari per gram, which provides a reasonable
/// balance between transaction confirmation speed and cost.
fn default_fee_per_gram() -> Option<MicroMinotari> {
    Some(MicroMinotari(5))
}

fn default_confirmation_window() -> Option<u64> {
    Some(REQUIRED_CONFIRMATIONS)
}

fn confirmation_window_schema() -> Schema {
    ObjectBuilder::new()
        .schema_type(SchemaType::new(Type::Integer))
        .default(Some(json!(REQUIRED_CONFIRMATIONS)))
        .description(Some("Number of confirmations required"))
        .build()
        .into()
}

/// Path parameters for wallet/account identification.
///
/// Used to extract the account name from URL path segments in account-related
/// endpoints.
///
/// # Example
///
/// For a request to `/accounts/my_wallet/balance`, the `name` field would
/// contain `"my_wallet"`.
#[derive(Debug, Deserialize, IntoParams, utoipa::ToSchema)]
pub struct WalletParams {
    /// The unique name identifying the wallet account.
    name: String,
}

/// Request body for locking funds in preparation for a transaction.
///
/// This request reserves (locks) a specified amount of funds from the account's
/// available UTXOs. Locked funds cannot be used in other transactions until
/// either the lock expires or the transaction is completed/cancelled.
///
/// # JSON Example
///
/// ```json
/// {
///   "amount": 1000000,
///   "num_outputs": 2,
///   "fee_per_gram": 5,
///   "estimated_output_size": 1024,
///   "seconds_to_lock_utxos": 3600,
///   "idempotency_key": "unique-request-id-12345"
/// }
/// ```
///
/// # Minimal Request
///
/// Only `amount` is required; all other fields have sensible defaults:
///
/// ```json
/// {
///   "amount": 1000000
/// }
/// ```
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct LockFundsRequest {
    /// The total amount to lock in MicroMinotari (1 Minotari = 1,000,000 MicroMinotari).
    ///
    /// This should cover the intended transaction amount plus any expected fees.
    #[schema(value_type = u64)]
    pub amount: MicroMinotari,

    /// Number of outputs to create in the transaction.
    ///
    /// Defaults to 1. Increase this when sending to multiple recipients or when
    /// the transaction requires multiple output UTXOs.
    #[serde(default = "default_num_outputs")]
    #[schema(default = "1")]
    pub num_outputs: Option<usize>,

    /// Fee per gram for the transaction in MicroMinotari.
    ///
    /// Defaults to 5 MicroMinotari. Higher fees may result in faster confirmation
    /// during periods of high network congestion.
    #[schema(value_type = u64)]
    #[serde(default = "default_fee_per_gram")]
    #[schema(default = "5")]
    pub fee_per_gram: Option<MicroMinotari>,

    /// Estimated size of each output in bytes.
    ///
    /// Used for fee calculation. If not provided, a default estimate is used
    /// based on standard output sizes.
    pub estimated_output_size: Option<usize>,

    /// Duration in seconds to keep the UTXOs locked.
    ///
    /// Defaults to 86,400 seconds (24 hours). After this period, locked UTXOs
    /// are automatically released if the transaction was not completed.
    #[serde(default = "default_seconds_to_lock_utxos")]
    #[schema(default = "86400")]
    pub seconds_to_lock_utxos: Option<u64>,

    /// Optional idempotency key to prevent duplicate requests.
    ///
    /// If provided, subsequent requests with the same key will return the
    /// cached result from the original request rather than locking additional
    /// funds.
    pub idempotency_key: Option<String>,

    /// Number of confirmations required before spending locked UTXOs.
    #[serde(default = "default_confirmation_window")]
    #[schema(schema_with = confirmation_window_schema)]
    pub confirmation_window: Option<u64>,
}

/// Represents a single recipient in a transaction request.
///
/// Each recipient specifies a destination address and the amount to send.
/// An optional payment ID can be included for tracking or identification
/// purposes.
///
/// # JSON Example
///
/// ```json
/// {
///   "address": "f4FxMqKAPDMqAjh6hTpC...",
///   "amount": 500000,
///   "payment_id": "invoice-2024-001"
/// }
/// ```
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct RecipientRequest {
    /// The recipient's Tari address in Base58 encoding.
    ///
    /// Must be a valid Tari address for the current network (mainnet, testnet, etc.).
    address: TariAddressBase58,

    /// The amount to send to this recipient in MicroMinotari.
    #[schema(value_type = u64)]
    amount: MicroMinotari,

    /// Optional payment identifier for transaction tracking.
    ///
    /// This can be used to associate the payment with an invoice, order, or
    /// other business reference. The payment ID is included in the transaction
    /// metadata.
    payment_id: Option<String>,
}

/// Request body for creating an unsigned transaction.
///
/// Creates a one-sided transaction that can be signed externally. This is
/// useful for cold wallet setups or multi-signature workflows where the
/// signing key is not available on the server.
///
/// # JSON Example
///
/// ```json
/// {
///   "recipients": [
///     {
///       "address": "f4FxMqKAPDMqAjh6hTpC...",
///       "amount": 500000,
///       "payment_id": "order-123"
///     },
///     {
///       "address": "f5GxNrLBPEMrBki7iTqD...",
///       "amount": 300000
///     }
///   ],
///   "seconds_to_lock_utxos": 7200,
///   "idempotency_key": "tx-request-abc123"
/// }
/// ```
///
/// # Notes
///
/// - The total amount sent equals the sum of all recipient amounts
/// - Transaction fees are calculated automatically based on transaction size
/// - UTXOs are locked during transaction creation to prevent double-spending
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct CreateTransactionRequest {
    /// List of recipients for the transaction.
    ///
    /// At least one recipient must be specified. Each recipient includes an
    /// address and amount, with an optional payment ID.
    recipients: Vec<RecipientRequest>,

    /// Duration in seconds to keep the input UTXOs locked.
    ///
    /// Defaults to 86,400 seconds (24 hours). The lock prevents the same UTXOs
    /// from being used in multiple transactions while the unsigned transaction
    /// is being signed and broadcast.

    #[serde(default = "default_seconds_to_lock_utxos")]
    #[schema(default = "86400")]
    seconds_to_lock_utxos: Option<u64>,

    /// Optional idempotency key to prevent duplicate transactions.
    ///
    /// If the same key is used in multiple requests, subsequent requests will
    /// return the original transaction rather than creating a new one.
    idempotency_key: Option<String>,

    #[serde(default = "default_confirmation_window")]
    #[schema(schema_with = confirmation_window_schema)]
    pub confirmation_window: Option<u64>,
}

/// Retrieves the current balance for a specified account.
///
/// Returns the account's available balance, pending incoming transactions,
/// and locked funds. This endpoint is useful for displaying wallet status
/// or checking available funds before initiating a transaction.
///
/// # Path Parameters
///
/// - `name`: The unique account name to query
///
/// # Response
///
/// Returns an [`AccountBalance`] object containing:
/// - Available (spendable) balance
/// - Pending incoming balance
/// - Locked balance (reserved for pending transactions)
///
/// # Errors
///
/// - [`ApiError::AccountNotFound`]: The specified account does not exist
/// - [`ApiError::DbError`]: Database connection or query failure
///
/// # Example Response
///
/// ```json
/// {
///   "available": 10000000,
///   "pending_incoming": 500000,
///   "locked": 200000
/// }
/// ```
#[utoipa::path(
    get,
    path = "/accounts/{name}/balance",
    responses(
        (status = 200, description = "Account balance retrieved successfully", body = AccountBalance),
        (status = 404, description = "Account not found", body = ApiError),
        (status = 500, description = "Internal server error", body = ApiError),
    ),
    params(
        ("name" = String, Path, description = "Name of the account to retrieve balance for"),
    )
)]
pub async fn api_get_balance(
    State(app_state): State<AppState>,
    Path(WalletParams { name }): Path<WalletParams>,
) -> Result<Json<AccountBalance>, ApiError> {
    let conn = app_state.db_pool.get().map_err(|e| ApiError::DbError(e.to_string()))?;
    let account = get_account_by_name(&conn, &name)?.ok_or_else(|| ApiError::AccountNotFound(name.clone()))?;

    let balance = get_balance(&conn, account.id)?;

    Ok(Json(balance))
}

/// Locks funds from an account for transaction preparation.
///
/// This endpoint reserves UTXOs totaling at least the requested amount,
/// preventing them from being used in other transactions. This is typically
/// the first step in creating a transaction, ensuring funds are available
/// and reserved before constructing the transaction.
///
/// # Path Parameters
///
/// - `name`: The account name to lock funds from
///
/// # Request Body
///
/// See [`LockFundsRequest`] for the complete request schema.
///
/// # Response
///
/// Returns a [`LockFundsResult`] containing:
/// - The selected UTXOs to use as inputs
/// - Whether a change output is required
/// - Total value of locked UTXOs
/// - Fee estimates with and without change
///
/// # Errors
///
/// - [`ApiError::AccountNotFound`]: The specified account does not exist
/// - [`ApiError::FailedToLockFunds`]: Insufficient funds or UTXO selection failure
/// - [`ApiError::DbError`]: Database connection or query failure
///
/// # Example
///
/// ```bash
/// curl -X POST http://localhost:8080/accounts/default/lock_funds \
///   -H "Content-Type: application/json" \
///   -d '{"amount": 1000000, "num_outputs": 1}'
/// ```
///
/// # Notes
///
/// - Locked UTXOs are automatically released after the configured timeout
/// - Use the `idempotency_key` to safely retry failed requests
/// - The actual locked amount may exceed the requested amount due to UTXO granularity
#[utoipa::path(
    post,
    path = "/accounts/{name}/lock_funds",
    request_body = LockFundsRequest,
    responses(
        (status = 200, description = "Funds locked successfully", body = LockFundsResult),
        (status = 400, description = "Bad request", body = ApiError),
        (status = 404, description = "Account not found", body = ApiError),
        (status = 500, description = "Internal server error", body = ApiError),
    ),
    params(
        ("name" = String, Path, description = "Name of the account to lock funds from"),
    )
)]
pub async fn api_lock_funds(
    State(app_state): State<AppState>,
    Path(WalletParams { name }): Path<WalletParams>,
    Json(body): Json<LockFundsRequest>,
) -> Result<Json<LockFundsResult>, ApiError> {
    let conn = app_state.db_pool.get().map_err(|e| ApiError::DbError(e.to_string()))?;
    let account = get_account_by_name(&conn, &name)?.ok_or_else(|| ApiError::AccountNotFound(name.clone()))?;

    let lock_amount = FundLocker::new(app_state.db_pool.clone());
    let response = lock_amount
        .lock(
            account.id,
            body.amount,
            body.num_outputs.expect("must be defaulted"),
            body.fee_per_gram.expect("must be defaulted"),
            body.estimated_output_size,
            body.idempotency_key,
            body.seconds_to_lock_utxos.expect("must be defaulted"),
            body.confirmation_window.expect("must be defaulted"),
        )
        .map_err(|e| ApiError::FailedToLockFunds(e.to_string()))?;
    Ok(Json(response))
}

/// Creates an unsigned one-sided transaction for external signing.
///
/// This endpoint constructs a complete transaction ready for signing, including
/// input selection, output creation, and fee calculation. The transaction is
/// returned in an unsigned state, allowing it to be signed by an external
/// key management system or hardware wallet.
///
/// # Path Parameters
///
/// - `name`: The account name to send funds from
///
/// # Request Body
///
/// See [`CreateTransactionRequest`] for the complete request schema.
///
/// # Response
///
/// Returns a JSON object containing the unsigned transaction data, including:
/// - Transaction inputs (selected UTXOs)
/// - Transaction outputs (recipient outputs and change)
/// - Fee information
/// - Data required for signing
///
/// # Errors
///
/// - [`ApiError::AccountNotFound`]: The specified account does not exist
/// - [`ApiError::FailedToLockFunds`]: Insufficient funds or UTXO selection failure
/// - [`ApiError::FailedCreateUnsignedTx`]: Transaction construction failure
/// - [`ApiError::DbError`]: Database connection or query failure
///
/// # Example
///
/// ```bash
/// curl -X POST http://localhost:8080/accounts/default/create_unsigned_transaction \
///   -H "Content-Type: application/json" \
///   -d '{
///     "recipients": [
///       {"address": "f4FxMqKAPDMqAjh6hTpC...", "amount": 1000000}
///     ]
///   }'
/// ```
///
/// # Workflow
///
/// 1. Client calls this endpoint to create an unsigned transaction
/// 2. Server locks required UTXOs and constructs the transaction
/// 3. Client signs the transaction externally
/// 4. Client broadcasts the signed transaction to the network
///
/// # Notes
///
/// - This creates a one-sided transaction (no recipient interaction required)
/// - UTXOs are automatically locked for the configured duration
/// - Fee is calculated at 5 MicroMinotari per gram
/// - Change outputs are created automatically when necessary
#[utoipa::path(
    post,
    path = "/accounts/{name}/create_unsigned_transaction",
    request_body = CreateTransactionRequest,
    responses(
        (status = 200, description = "Unsigned transaction created successfully", body = JsonValue),
        (status = 400, description = "Bad request", body = ApiError),
        (status = 404, description = "Account not found", body = ApiError),
        (status = 500, description = "Internal server error", body = ApiError),
    ),
    params(
        ("name" = String, Path, description = "Name of the account to create transaction for"),
    )
)]
pub async fn api_create_unsigned_transaction(
    State(app_state): State<AppState>,
    Path(WalletParams { name }): Path<WalletParams>,
    Json(body): Json<CreateTransactionRequest>,
) -> Result<Json<JsonValue>, ApiError> {
    let recipients: Vec<Recipient> = body
        .recipients
        .into_iter()
        .map(|r| Recipient {
            address: r.address.0,
            amount: r.amount,
            payment_id: r.payment_id,
        })
        .collect();

    let conn = app_state.db_pool.get().map_err(|e| ApiError::DbError(e.to_string()))?;
    let account = get_account_by_name(&conn, &name)?.ok_or_else(|| ApiError::AccountNotFound(name.clone()))?;

    let amount = recipients.iter().map(|r| r.amount).sum();
    let num_outputs = recipients.len();
    let fee_per_gram = MicroMinotari(5);
    let estimated_output_size = None;
    let seconds_to_lock_utxos = body.seconds_to_lock_utxos.unwrap_or(86400); // 24 hours

    let lock_amount = FundLocker::new(app_state.db_pool.clone());
    let locked_funds = lock_amount
        .lock(
            account.id,
            amount,
            num_outputs,
            fee_per_gram,
            estimated_output_size,
            body.idempotency_key,
            seconds_to_lock_utxos,
            body.confirmation_window.expect("must be defaulted"),
        )
        .map_err(|e| ApiError::FailedToLockFunds(e.to_string()))?;

    let one_sided_tx =
        OneSidedTransaction::new(app_state.db_pool.clone(), app_state.network, app_state.password.clone());
    let result = one_sided_tx
        .create_unsigned_transaction(&account, locked_funds, recipients, fee_per_gram)
        .map_err(|e| ApiError::FailedCreateUnsignedTx(e.to_string()))?;

    Ok(Json(serde_json::to_value(result)?))
}
