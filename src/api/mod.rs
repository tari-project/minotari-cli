use axum::{Router, extract::FromRef, routing::get, routing::post};
use sqlx::SqlitePool;
use tari_common::configuration::Network;
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

mod accounts;
mod error;
pub mod types;

#[derive(Clone)]
pub struct AppState {
    pub db_pool: SqlitePool,
    pub network: Network,
    pub password: String,
}

impl FromRef<AppState> for SqlitePool {
    fn from_ref(state: &AppState) -> Self {
        state.db_pool.clone()
    }
}

#[derive(OpenApi)]
#[openapi(
    paths(
        accounts::api_get_balance,
    ),
    components(
        schemas(
            crate::db::AccountBalance,
            error::ApiError,
            accounts::WalletParams,
        )
    ),
    tags(
        (name = "minotari-cli", description = "Minotari CLI API"),
    )
)]
pub struct ApiDoc;

pub fn create_router(db_pool: SqlitePool, network: Network, password: String) -> Router {
    let app_state = AppState {
        db_pool,
        network,
        password,
    };

    Router::new()
        .merge(SwaggerUi::new("/swagger-ui").url("/openapi.json", ApiDoc::openapi()))
        .route("/accounts/{name}/balance", get(accounts::api_get_balance))
        .route("/accounts/{name}/lock_funds", post(accounts::api_lock_funds))
        .with_state(app_state)
}
