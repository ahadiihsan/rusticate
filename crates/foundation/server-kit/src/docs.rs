use axum::Router;
use axum::routing::get;
use utoipa::openapi::OpenApi;
use utoipa_scalar::{Scalar, Servable as _};

/// `/openapi.json`, plus the interactive reference at `/docs` when enabled.
pub fn docs_router(openapi: OpenApi, ui: bool) -> Router {
    let spec = openapi.clone();
    let router = Router::new().route(
        "/openapi.json",
        get(move || {
            let spec = spec.clone();
            async move { api_kit::Json(spec) }
        }),
    );
    if ui {
        router.merge(Scalar::with_url("/docs", openapi))
    } else {
        router
    }
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt as _;
    use utoipa::openapi::OpenApiBuilder;

    use super::*;

    async fn status(router: Router, uri: &str) -> StatusCode {
        let request = Request::get(uri).body(Body::empty()).unwrap();
        router.oneshot(request).await.unwrap().status()
    }

    #[tokio::test]
    async fn serves_the_document_always_and_the_ui_only_when_enabled() {
        let document = OpenApiBuilder::new().build();

        assert_eq!(
            status(docs_router(document.clone(), false), "/openapi.json").await,
            StatusCode::OK
        );
        assert_eq!(
            status(docs_router(document.clone(), false), "/docs").await,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            status(docs_router(document, true), "/docs").await,
            StatusCode::OK
        );
    }
}
