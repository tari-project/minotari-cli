use axum::{
    Json,
    extract::{Path, State},
};
use serde_json::Value as JsonValue;
use utoipa::IntoParams;

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

fn default_seconds_to_lock_utxos() -> Option<u64> {
    Some(86400)
}

fn default_num_outputs() -> Option<usize> {
    Some(1)
}

fn default_fee_per_gram() -> Option<MicroMinotari> {
    Some(MicroMinotari(5))
}

fn default_confirmation_window() -> Option<u64> {
    Some(REQUIRED_CONFIRMATIONS)
}

#[derive(Debug, serde::Deserialize, IntoParams, utoipa::ToSchema)]
pub struct WalletParams {
    name: String,
}

#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct LockFundsRequest {
    #[schema(value_type = u64)]
    pub amount: MicroMinotari,

    #[serde(default = "default_num_outputs")]
    #[schema(default = "1")]
    pub num_outputs: Option<usize>,

    #[schema(value_type = u64)]
    #[serde(default = "default_fee_per_gram")]
    #[schema(default = "5")]
    pub fee_per_gram: Option<MicroMinotari>,

    pub estimated_output_size: Option<usize>,

    #[serde(default = "default_seconds_to_lock_utxos")]
    #[schema(default = "86400")]
    pub seconds_to_lock_utxos: Option<u64>,

    pub idempotency_key: Option<String>,

    #[serde(default = "default_confirmation_window")]
    #[schema(default = "3")]
    pub confirmation_window: Option<u64>,
}

#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct RecipientRequest {
    address: TariAddressBase58,
    #[schema(value_type = u64)]
    amount: MicroMinotari,
    payment_id: Option<String>,
}

#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct CreateTransactionRequest {
    recipients: Vec<RecipientRequest>,

    #[serde(default = "default_seconds_to_lock_utxos")]
    #[schema(default = "86400")]
    seconds_to_lock_utxos: Option<u64>,

    idempotency_key: Option<String>,

    #[serde(default = "default_confirmation_window")]
    #[schema(default = "3")]
    pub confirmation_window: Option<u64>,
}

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
    let mut conn = app_state.db_pool.acquire().await?;
    let account = get_account_by_name(&mut conn, &name)
        .await?
        .ok_or_else(|| ApiError::AccountNotFound(name.clone()))?;

    let balance = get_balance(&mut conn, account.id).await?;

    Ok(Json(balance))
}

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
    let mut conn = app_state.db_pool.acquire().await?;
    let account = get_account_by_name(&mut conn, &name)
        .await?
        .ok_or_else(|| ApiError::AccountNotFound(name.clone()))?;

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
        .await
        .map_err(|e| ApiError::FailedToLockFunds(e.to_string()))?;
    Ok(Json(response))
}

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

    let mut conn = app_state.db_pool.acquire().await?;
    let account = get_account_by_name(&mut conn, &name)
        .await?
        .ok_or_else(|| ApiError::AccountNotFound(name.clone()))?;

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
        .await
        .map_err(|e| ApiError::FailedToLockFunds(e.to_string()))?;

    let one_sided_tx =
        OneSidedTransaction::new(app_state.db_pool.clone(), app_state.network, app_state.password.clone());
    let result = one_sided_tx
        .create_unsigned_transaction(&account, locked_funds, recipients, fee_per_gram)
        .await
        .map_err(|e| ApiError::FailedCreateUnsignedTx(e.to_string()))?;

    Ok(Json(serde_json::to_value(result)?))
}
