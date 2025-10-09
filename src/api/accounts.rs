use axum::{
    Json,
    extract::{Path, State},
};
use utoipa::IntoParams;

use super::error::ApiError;
use crate::{
    api::AppState,
    db::{AccountBalance, get_account_by_name, get_balance},
};

#[derive(Debug, serde::Deserialize, IntoParams)]
pub struct GetBalanceParams {
    name: String,
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
    Path(GetBalanceParams { name }): Path<GetBalanceParams>,
) -> Result<Json<AccountBalance>, ApiError> {
    let account = get_account_by_name(&app_state.db_pool, &name)
        .await?
        .ok_or_else(|| ApiError::AccountNotFound(name.clone()))?;

    let balance = get_balance(&app_state.db_pool, account.id).await?;

    Ok(Json(balance))
}
