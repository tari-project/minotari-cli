//! RESTful HTTP API for wallet operations.
//!
//! This module provides a web API for interacting with the Minotari wallet, including
//! balance queries, fund locking, and unsigned transaction creation. The API is documented
//! using OpenAPI (Swagger) specifications and includes interactive documentation.
//!
//! # API Endpoints
//!
//! The API exposes the following endpoints:
//!
//! - `GET /accounts/{name}/balance` - Retrieve account balance
//! - `POST /accounts/{name}/lock_funds` - Lock UTXOs for transaction creation
//! - `POST /accounts/{name}/create_unsigned_transaction` - Create an unsigned one-sided transaction
//! - `GET /swagger-ui` - Interactive Swagger UI documentation
//! - `GET /openapi.json` - OpenAPI specification in JSON format
//!
//! # OpenAPI Documentation
//!
//! The API is fully documented using the OpenAPI 3.0 specification via the `utoipa` crate.
//! All endpoints, request/response schemas, and error types are automatically included in
//! the generated documentation accessible through Swagger UI.
//!
//! # Usage Example
//!
//! ```no_run
//! use minotari::api::create_router;
//! use minotari::init_db;
//! use tari_common::configuration::Network;
//!
//! # async fn example() -> anyhow::Result<()> {
//! let db_pool = init_db("wallet.db").await?;
//! let network = Network::Esmeralda;
//! let password = "secure_password".to_string();
//!
//! let router = create_router(db_pool, network, password);
//!
//! // Serve with axum
//! let listener = tokio::net::TcpListener::bind("127.0.0.1:3000").await?;
//! axum::serve(listener, router).await?;
//! # Ok(())
//! # }
//! ```
//!
//! # Security Considerations
//!
//! - The password stored in [`AppState`] is used to decrypt wallet keys for transaction operations
//! - Fund locking prevents double-spending by temporarily reserving UTXOs
//! - Idempotency keys can be used to prevent duplicate operations
//! - All API errors are properly typed and do not leak sensitive information

use axum::{Router, extract::FromRef, routing::get, routing::post};
use sqlx::SqlitePool;
use tari_common::configuration::Network;
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;
use zeroize::Zeroizing;

pub mod accounts;
mod error;
pub mod types;

/// Application state shared across all API handlers.
///
/// This state is cloned for each request and provides access to the database,
/// network configuration, and wallet password for decrypting keys.
///
/// # Fields
///
/// * `db_pool` - SQLite connection pool for database operations
/// * `network` - Tari network configuration (Esmeralda, Nextnet, Mainnet, etc.)
/// * `password` - Password for decrypting wallet keys (securely zeroized on drop)
#[derive(Clone)]
pub struct AppState {
    pub db_pool: SqlitePool,
    pub network: Network,
    pub password: Zeroizing<String>,
}

impl FromRef<AppState> for SqlitePool {
    fn from_ref(state: &AppState) -> Self {
        state.db_pool.clone()
    }
}

/// OpenAPI documentation structure for the Minotari wallet API.
///
/// This struct is used by `utoipa` to generate the complete OpenAPI specification,
/// which includes all API endpoints, request/response schemas, and component definitions.
///
/// The generated specification is served at `/openapi.json` and powers the Swagger UI
/// at `/swagger-ui`.
///
/// # Registered Components
///
/// ## Paths (Endpoints)
/// - `/accounts/{name}/balance` - Get account balance
/// - `/accounts/{name}/lock_funds` - Lock funds for transaction
/// - `/accounts/{name}/create_unsigned_transaction` - Create unsigned transaction
///
/// ## Schemas
/// - `AccountBalance` - Balance information with available/pending amounts
/// - `ApiError` - Standardized error responses
/// - `WalletParams` - Account name path parameter
/// - `LockFundsRequest` - Request body for fund locking
/// - `CreateTransactionRequest` - Request body for transaction creation
/// - `RecipientRequest` - Transaction recipient details
/// - `LockFundsResult` - Response from fund locking operation
/// - `TariAddressBase58` - Base58-encoded Tari address
#[derive(OpenApi)]
#[openapi(
    paths(
        accounts::api_get_balance,
        accounts::api_lock_funds,
        accounts::api_create_unsigned_transaction,
    ),
    components(
        schemas(
            crate::db::AccountBalance,
            error::ApiError,
            accounts::WalletParams,
            accounts::LockFundsRequest,
            accounts::CreateTransactionRequest,
            accounts::RecipientRequest,
            crate::api::types::LockFundsResult,
            crate::api::types::TariAddressBase58,
        )
    ),
    tags(
        (name = "minotari-cli", description = "Minotari CLI API"),
    )
)]
pub struct ApiDoc;

/// Creates and configures the API router with all endpoints and middleware.
///
/// This function sets up the complete Axum router with:
/// - All API endpoints for account operations
/// - Swagger UI at `/swagger-ui` for interactive API documentation
/// - OpenAPI specification at `/openapi.json`
/// - Shared application state containing database pool, network, and password
///
/// # Parameters
///
/// * `db_pool` - SQLite connection pool for database access
/// * `network` - Tari network configuration (Esmeralda, Nextnet, Mainnet, etc.)
/// * `password` - Password for decrypting wallet keys (kept in memory for API operations)
///
/// # Returns
///
/// An Axum `Router` ready to be served with `axum::serve()`.
///
/// # Example
///
/// ```no_run
/// use minotari::api::create_router;
/// use minotari::init_db;
/// use tari_common::configuration::Network;
///
/// # async fn example() -> anyhow::Result<()> {
/// let db_pool = init_db("wallet.db").await?;
/// let router = create_router(db_pool, Network::Esmeralda, "password".to_string());
///
/// let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await?;
/// axum::serve(listener, router).await?;
/// # Ok(())
/// # }
/// ```
pub fn create_router(db_pool: SqlitePool, network: Network, password: String) -> Router {
    let app_state = AppState {
        db_pool,
        network,
        password: Zeroizing::new(password),
    };

    Router::new()
        .merge(SwaggerUi::new("/swagger-ui").url("/openapi.json", ApiDoc::openapi()))
        .route("/accounts/{name}/balance", get(accounts::api_get_balance))
        .route("/accounts/{name}/lock_funds", post(accounts::api_lock_funds))
        .route(
            "/accounts/{name}/create_unsigned_transaction",
            post(accounts::api_create_unsigned_transaction),
        )
        .with_state(app_state)
}
