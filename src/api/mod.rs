use axum::{Router, extract::FromRef, routing::get};
use sqlx::SqlitePool;
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

mod accounts;
mod error;

#[derive(Clone)]
pub struct AppState {
    pub db_pool: SqlitePool,
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
        )
    ),
    tags(
        (name = "minotari-cli", description = "Minotari CLI API"),
    )
)]
struct ApiDoc;

pub fn create_router(db_pool: SqlitePool) -> Router {
    let app_state = AppState { db_pool };

    Router::new()
        .merge(SwaggerUi::new("/swagger-ui").url("/openapi.json", ApiDoc::openapi()))
        .route("/accounts/{name}/balance", get(accounts::api_get_balance))
        .with_state(app_state)
}
