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
}

impl From<sqlx::Error> for ApiError {
    fn from(err: sqlx::Error) -> Self {
        ApiError::DbError(err.to_string())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, error_message) = match self {
            ApiError::InternalServerError(msg) => (StatusCode::INTERNAL_SERVER_ERROR, msg),
            ApiError::DbError(e) => (StatusCode::INTERNAL_SERVER_ERROR, e),
            ApiError::AccountNotFound(name) => (StatusCode::NOT_FOUND, format!("Account '{}' not found", name)),
        };

        let body = Json(json!({
            "error": error_message,
        }));

        (status, body).into_response()
    }
}
