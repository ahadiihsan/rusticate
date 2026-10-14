use std::any::Any;

use api_kit::{ApiError, RequestScope};
use axum::Router;
use axum::error_handling::HandleErrorLayer;
use axum::extract::{DefaultBodyLimit, MatchedPath, Request};
use axum::http::{HeaderName, HeaderValue, Method, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use tower::ServiceBuilder;
use tower::load_shed::error::Overloaded;
use tower::timeout::error::Elapsed;
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::compression::CompressionLayer;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::sensitive_headers::SetSensitiveRequestHeadersLayer;
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::trace::{DefaultOnResponse, TraceLayer};
use tracing::Level;
use uuid::Uuid;

use crate::HttpConfig;

pub const REQUEST_ID_HEADER: HeaderName = HeaderName::from_static("x-request-id");

/// Wraps `router` in the standard middleware stack (outermost first):
///
/// 1. request id + [`RequestScope`] (problem `instance` / `request_id`)
/// 2. tracing span per request, continuing an inbound `traceparent`
/// 3. sensitive request headers redacted from logs
/// 4. panics → 500 problem (never a dropped connection)
/// 5. load shedding → 503 problem when `max_concurrent_requests` is reached
/// 6. request deadline → 503 problem
/// 7. body limit, compression, CORS, security headers
pub fn with_middleware(router: Router, cfg: &HttpConfig) -> Router {
    let router = router
        .fallback(|| async { ApiError::not_found("route") })
        .method_not_allowed_fallback(|| async {
            ApiError::Rejected {
                status: StatusCode::METHOD_NOT_ALLOWED,
                code: "METHOD_NOT_ALLOWED",
                detail: "The HTTP method is not allowed for this resource.".to_owned(),
            }
        });

    let router = match cors_layer(cfg) {
        Some(cors) => router.layer(cors),
        None => router,
    };

    router.layer(
        ServiceBuilder::new()
            .layer(middleware::from_fn(request_context))
            .layer(
                TraceLayer::new_for_http()
                    .make_span_with(make_span)
                    .on_response(DefaultOnResponse::new().level(Level::INFO)),
            )
            .layer(SetSensitiveRequestHeadersLayer::new([
                header::AUTHORIZATION,
                header::COOKIE,
                HeaderName::from_static("idempotency-key"),
            ]))
            .layer(CatchPanicLayer::custom(panic_response))
            .layer(HandleErrorLayer::new(overload_or_timeout))
            .load_shed()
            .concurrency_limit(cfg.max_concurrent_requests)
            .timeout(cfg.request_timeout())
            .layer(DefaultBodyLimit::max(cfg.body_limit_bytes))
            .layer(CompressionLayer::new())
            .layer(SetResponseHeaderLayer::overriding(
                header::X_CONTENT_TYPE_OPTIONS,
                HeaderValue::from_static("nosniff"),
            ))
            .layer(SetResponseHeaderLayer::if_not_present(
                header::CACHE_CONTROL,
                HeaderValue::from_static("no-store"),
            )),
    )
}

/// Accepts a client `x-request-id` only if it is a UUID (never echo arbitrary
/// client input into logs); otherwise mints a UUIDv7.
async fn request_context(mut request: Request, next: Next) -> Response {
    let request_id = request
        .headers()
        .get(&REQUEST_ID_HEADER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| Uuid::parse_str(value).ok())
        .unwrap_or_else(Uuid::now_v7);
    let header_value = HeaderValue::from_str(&request_id.to_string()).ok();
    if let Some(value) = &header_value {
        request
            .headers_mut()
            .insert(REQUEST_ID_HEADER, value.clone());
    }
    let scope = RequestScope {
        request_id,
        path: request.uri().path().to_owned(),
    };

    let mut response = scope.run(next.run(request)).await;
    if let Some(value) = header_value {
        response.headers_mut().insert(REQUEST_ID_HEADER, value);
    }
    response
}

fn make_span(request: &Request) -> tracing::Span {
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map_or("<unmatched>", MatchedPath::as_str);
    let request_id = request
        .headers()
        .get(&REQUEST_ID_HEADER)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let span = tracing::info_span!(
        "http.request",
        otel.kind = "server",
        otel.name = %format_args!("{} {route}", request.method()),
        http.request.method = %request.method(),
        http.route = route,
        request_id,
        tenant_id = tracing::field::Empty,
        actor_id = tracing::field::Empty,
    );
    telemetry::extract_parent(&span, request.headers());
    span
}

fn panic_response(_panic: Box<dyn Any + Send + 'static>) -> Response {
    // The panic message is logged by the telemetry panic hook; it may contain
    // data, so it is never part of the response.
    ApiError::internal(std::io::Error::other("request handler panicked")).into_response()
}

async fn overload_or_timeout(err: tower::BoxError) -> ApiError {
    if err.is::<Overloaded>() {
        ApiError::unavailable(
            "OVERLOADED",
            "The service is at capacity. Retry the request.",
        )
    } else if err.is::<Elapsed>() {
        ApiError::unavailable(
            "REQUEST_TIMEOUT",
            "The request took too long. Retry the request.",
        )
    } else {
        ApiError::Internal(err)
    }
}

fn cors_layer(cfg: &HttpConfig) -> Option<CorsLayer> {
    let origins: Vec<HeaderValue> = cfg
        .cors_allowed_origins
        .iter()
        .filter_map(|origin| HeaderValue::from_str(origin).ok())
        .collect();
    if origins.is_empty() {
        return None;
    }
    Some(
        CorsLayer::new()
            .allow_origin(AllowOrigin::list(origins))
            .allow_methods([
                Method::GET,
                Method::POST,
                Method::PUT,
                Method::PATCH,
                Method::DELETE,
            ])
            .allow_headers([
                header::AUTHORIZATION,
                header::CONTENT_TYPE,
                HeaderName::from_static("idempotency-key"),
                REQUEST_ID_HEADER,
            ])
            .expose_headers([REQUEST_ID_HEADER, header::RETRY_AFTER, header::ETAG])
            .max_age(std::time::Duration::from_hours(1)),
    )
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use axum::body::Body;
    use axum::http::{HeaderMap, Request as HttpRequest};
    use axum::routing::get;
    use http_body_util::BodyExt as _;
    use tower::ServiceExt as _;

    use super::*;

    fn app(cfg: &HttpConfig) -> Router {
        let router = Router::new()
            .route("/ok", get(|| async { "ok" }))
            .route(
                "/slow",
                get(|| async {
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    "late"
                }),
            )
            .route("/panic", get(boom));
        with_middleware(router, cfg)
    }

    async fn boom() -> &'static str {
        panic!("boom: secret=hunter2")
    }

    async fn call(
        cfg: &HttpConfig,
        uri: &str,
        request_id: Option<&str>,
    ) -> (StatusCode, HeaderMap, String) {
        let mut request = HttpRequest::get(uri);
        if let Some(id) = request_id {
            request = request.header(&REQUEST_ID_HEADER, id);
        }
        let response = app(cfg)
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        (status, headers, String::from_utf8_lossy(&body).into_owned())
    }

    #[tokio::test]
    async fn echoes_valid_request_ids_and_replaces_invalid_ones() {
        let cfg = HttpConfig::default();
        let valid = "0192f0c1-5b7e-7cc0-8b5e-4f1a2b3c4d5e";

        let (_, kept, _) = call(&cfg, "/ok", Some(valid)).await;
        let (_, replaced, _) = call(&cfg, "/ok", Some("<script>")).await;

        assert_eq!(kept[&REQUEST_ID_HEADER], valid);
        assert!(Uuid::parse_str(replaced[&REQUEST_ID_HEADER].to_str().unwrap()).is_ok());
    }

    #[tokio::test]
    async fn unknown_route_is_problem_404_with_instance() {
        let (status, headers, body) = call(&HttpConfig::default(), "/nope", None).await;

        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(headers[header::CONTENT_TYPE], api_kit::PROBLEM_JSON);
        assert!(body.contains(r#""instance":"/nope""#));
        assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    }

    #[tokio::test]
    async fn panics_become_problem_500_without_the_message() {
        let (status, _, body) = call(&HttpConfig::default(), "/panic", None).await;

        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(!body.contains("hunter2"));
    }

    #[tokio::test]
    async fn slow_requests_time_out_as_problem_503() {
        let cfg = HttpConfig {
            request_timeout_ms: 50,
            ..HttpConfig::default()
        };

        let (status, headers, body) = call(&cfg, "/slow", None).await;

        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(headers[header::RETRY_AFTER], "1");
        assert!(body.contains("REQUEST_TIMEOUT"));
    }
}
