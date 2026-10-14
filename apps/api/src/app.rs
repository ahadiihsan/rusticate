//! Router assembly. The only place bounded contexts are mounted.

use std::sync::Arc;

use axum::Router;
use axum::extract::FromRef;
use datastore::DataStore;
use iam::TokenVerifier;
use server_kit::{HttpConfig, Readiness};
use utoipa::OpenApi as _;
use utoipa_axum::router::OpenApiRouter;

use crate::openapi::ApiDoc;

/// Shared application state. Contexts extract what they need via `FromRef`.
#[derive(Debug, Clone, FromRef)]
pub struct AppState {
    pub db: DataStore,
}

/// Every mounted context's routes plus the merged OpenAPI document.
fn routes() -> (Router<AppState>, utoipa::openapi::OpenApi) {
    OpenApiRouter::with_openapi(ApiDoc::openapi())
        // xtask:contexts
        .merge(billing::router())
        .split_for_parts()
}

/// The complete, production router.
pub fn build(
    state: AppState,
    verifier: Arc<TokenVerifier>,
    readiness: &Readiness,
    http: &HttpConfig,
) -> Router {
    let (api, openapi) = routes();
    let api = api
        .with_state(state)
        .layer(axum::middleware::from_fn_with_state(
            verifier,
            iam::authenticate,
        ));
    let router = api
        .merge(readiness.router())
        .merge(server_kit::docs_router(openapi, http.docs_ui));
    server_kit::with_middleware(router, http)
}

/// The OpenAPI document as pretty JSON (the committed contract, API-004).
pub fn openapi_json() -> Result<String, serde_json::Error> {
    let (_, openapi) = routes();
    let mut json = serde_json::to_string_pretty(&openapi)?;
    json.push('\n');
    Ok(json)
}
