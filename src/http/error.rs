use thiserror::Error;

#[derive(Debug, Error)]
pub enum HttpError {
    #[error("Request failed: {0}")]
    RequestFailed(#[from] reqwest::Error),

    #[error("Middleware error: {0}")]
    MiddlewareError(#[from] reqwest_middleware::Error),

    #[error("Server error {status}: {body}")]
    ServerError { status: reqwest::StatusCode, body: String },

    #[error("URL parse error: {0}")]
    UrlError(#[from] url::ParseError),

    #[error("Unsupported HTTP method")]
    UnsupportedMethod,

    #[error("JSON serialization error: {0}")]
    JsonError(#[from] serde_json::Error),
}
