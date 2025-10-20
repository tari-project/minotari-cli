use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;
use thiserror::Error;
use utoipa::ToSchema;

#[derive(Debug, Error, ToSchema)]
pub enum ApiError {
    #[error("Internal server error: {0}")]
    #[allow(dead_code)]
    InternalServerError(String),
    #[error("Database error: {0}")]
    DbError(String),
    #[error("Account not found: {0}")]
    AccountNotFound(String),
    #[error("Failed to create an unsigned transaction: {0}")]
    FailedCreateUnsignedTx(String),
}

impl From<sqlx::Error> for ApiError {
    fn from(err: sqlx::Error) -> Self {
        ApiError::DbError(err.to_string())
    }
}

impl From<serde_json::Error> for ApiError {
    fn from(err: serde_json::Error) -> Self {
        ApiError::InternalServerError(format!("JSON serialization error: {}", err))
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, error_message) = match self {
            ApiError::InternalServerError(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg),
            ApiError::DbError(e) => (StatusCode::INTERNAL_SERVER_ERROR, e),
            ApiError::AccountNotFound(name) => (StatusCode::NOT_FOUND, format!("Account '{}' not found", name)),
            ApiError::FailedCreateUnsignedTx(e) => (StatusCode::INTERNAL_SERVER_ERROR, e),
        };

        let body = Json(json!({
            "error": error_message,
        }));

        (status, body).into_response()
    }
}
