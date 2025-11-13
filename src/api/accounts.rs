use axum::{
    Json,
    extract::{Path, State},
};
use utoipa::IntoParams;

use super::error::ApiError;
use crate::{
    api::{AppState, types::LockFundsResponse},
    db::{AccountBalance, get_account_by_name, get_balance},
    transactions::lock_amount::LockAmount,
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

#[derive(Debug, serde::Deserialize, IntoParams, utoipa::ToSchema)]
pub struct WalletParams {
    name: String,
}

#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct LockFundsRequest {
    #[schema(value_type = u64)]
    amount: MicroMinotari,

    #[serde(default = "default_num_outputs")]
    #[schema(default = "1")]
    num_outputs: Option<usize>,

    #[schema(value_type = u64)]
    #[serde(default = "default_fee_per_gram")]
    #[schema(default = "5")]
    fee_per_gram: Option<MicroMinotari>,

    estimated_output_size: Option<usize>,

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

#[axum::debug_handler]
pub async fn api_lock_funds(
    State(app_state): State<AppState>,
    Path(WalletParams { name }): Path<WalletParams>,
    Json(body): Json<LockFundsRequest>,
) -> Result<Json<LockFundsResponse>, ApiError> {
    let mut conn = app_state.db_pool.acquire().await?;
    let account = get_account_by_name(&mut conn, &name)
        .await?
        .ok_or_else(|| ApiError::AccountNotFound(name.clone()))?;

    let lock_amount = LockAmount::new(app_state.db_pool.clone());
    let response = lock_amount
        .lock(
            account,
            body.amount,
            body.num_outputs.expect("must be defaulted"),
            body.fee_per_gram.expect("must be defaulted"),
            body.estimated_output_size,
            body.idempotency_key,
            body.seconds_to_lock_utxos.expect("must be defaulted"),
        )
        .await
        .map_err(|e| ApiError::FailedToLockFunds(e.to_string()))?;
    Ok(Json(response))
}
