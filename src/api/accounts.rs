use axum::{
    Json,
    extract::{Path, State},
};
use serde_json::Value as JsonValue;
use utoipa::IntoParams;

use super::error::ApiError;
use crate::{
    api::{AppState, types::TariAddressBase58},
    db::{AccountBalance, get_account_by_name, get_balance},
    transactions::one_sided_transaction::{OneSidedTransaction, Recipient},
};
use tari_transaction_components::tari_amount::MicroMinotari;

fn default_seconds_to_lock_utxos() -> Option<u64> {
    Some(86400)
}

#[derive(Debug, serde::Deserialize, IntoParams, utoipa::ToSchema)]
pub struct WalletParams {
    name: String,
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

    let seconds_to_lock_utxos = body.seconds_to_lock_utxos;

    let mut conn = app_state.db_pool.acquire().await?;
    let account = get_account_by_name(&mut conn, &name)
        .await?
        .ok_or_else(|| ApiError::AccountNotFound(name.clone()))?;

    let one_sided_tx =
        OneSidedTransaction::new(app_state.db_pool.clone(), app_state.network, app_state.password.clone());
    let result = one_sided_tx
        .create_unsigned_transaction(
            account,
            recipients,
            body.idempotency_key,
            seconds_to_lock_utxos.unwrap(),
        )
        .await
        .map_err(|e| ApiError::FailedCreateUnsignedTx(e.to_string()))?;

    Ok(Json(serde_json::to_value(result)?))
}
