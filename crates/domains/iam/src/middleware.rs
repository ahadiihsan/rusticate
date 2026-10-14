use std::sync::Arc;

use api_kit::ApiError;
use axum::extract::{Request, State};
use axum::http::header;
use axum::middleware::Next;
use axum::response::{IntoResponse as _, Response};
use op_context::CorrelationId;
use uuid::Uuid;

use crate::TokenVerifier;

/// Authentication middleware (SEC-002). Mount with
/// `axum::middleware::from_fn_with_state(verifier, iam::authenticate)`.
///
/// * No `Authorization` header: the request continues anonymously; handlers
///   that take `OpCtx` reject it with 401, public handlers serve it.
/// * Present but invalid: 401 immediately (never silently anonymous).
/// * Valid: the minted `OpCtx` is inserted into the request extensions and the
///   tenant/actor are recorded on the request span.
pub async fn authenticate(
    State(verifier): State<Arc<TokenVerifier>>,
    mut request: Request,
    next: Next,
) -> Response {
    let Some(authorization) = request.headers().get(header::AUTHORIZATION) else {
        return next.run(request).await;
    };
    let Some(token) = bearer_token(authorization) else {
        return ApiError::Unauthenticated.into_response();
    };
    let correlation_id = request
        .headers()
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| Uuid::parse_str(value).ok())
        .map_or_else(CorrelationId::new_v7, CorrelationId::from_uuid);

    match verifier.verify(token, correlation_id) {
        Ok(ctx) => {
            let span = tracing::Span::current();
            span.record("tenant_id", tracing::field::display(ctx.tenant_id()));
            span.record("actor_id", tracing::field::display(ctx.actor_id()));
            request.extensions_mut().insert(ctx);
            next.run(request).await
        }
        Err(err) => {
            tracing::debug!(error = %err, "bearer token rejected");
            ApiError::Unauthenticated.into_response()
        }
    }
}

/// `Bearer <token>`; the scheme is case-insensitive (RFC 9110 §11.1).
fn bearer_token(value: &axum::http::HeaderValue) -> Option<&str> {
    let (scheme, token) = value.to_str().ok()?.split_once(' ')?;
    let token = token.trim();
    (scheme.eq_ignore_ascii_case("bearer") && !token.is_empty()).then_some(token)
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::body::Body;
    use axum::http::StatusCode;
    use axum::routing::get;
    use op_context::{ActorId, OpCtx, TenantId};
    use secrecy::SecretString;
    use tower::ServiceExt as _;

    use super::*;
    use crate::{AuthConfig, dev};

    const SECRET: &str = "0123456789abcdef0123456789abcdef";

    fn app() -> Router {
        let verifier = Arc::new(
            TokenVerifier::new(&AuthConfig {
                issuer: "iss".to_owned(),
                audience: "aud".to_owned(),
                public_key_pem: None,
                public_key_algorithm: crate::config::PublicKeyAlgorithm::default(),
                hs256_secret: Some(SecretString::from(SECRET.to_owned())),
                leeway_secs: 0,
            })
            .unwrap(),
        );
        Router::new()
            .route(
                "/private",
                get(|ctx: OpCtx| async move { ctx.tenant_id().to_string() }),
            )
            .route("/public", get(|| async { "hello" }))
            .layer(axum::middleware::from_fn_with_state(verifier, authenticate))
    }

    async fn status(uri: &str, authorization: Option<String>) -> StatusCode {
        let mut request = axum::http::Request::get(uri);
        if let Some(value) = authorization {
            request = request.header(header::AUTHORIZATION, value);
        }
        app()
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap()
            .status()
    }

    fn valid_token() -> String {
        dev::issue_hs256(&dev::DevToken {
            secret: SECRET,
            issuer: "iss",
            audience: "aud",
            tenant: TenantId::from_uuid(Uuid::now_v7()),
            actor: ActorId::from_uuid(Uuid::now_v7()),
            permissions: &[],
            ttl_secs: 60,
        })
        .unwrap()
    }

    #[tokio::test]
    async fn anonymous_requests_reach_public_routes_only() {
        assert_eq!(status("/public", None).await, StatusCode::OK);
        assert_eq!(status("/private", None).await, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn invalid_credentials_are_rejected_even_on_public_routes() {
        assert_eq!(
            status("/public", Some("Bearer garbage".to_owned())).await,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            status("/public", Some("Basic dXNlcjpwYXNz".to_owned())).await,
            StatusCode::UNAUTHORIZED
        );
    }

    #[tokio::test]
    async fn valid_bearer_token_authenticates() {
        assert_eq!(
            status("/private", Some(format!("bearer {}", valid_token()))).await,
            StatusCode::OK
        );
    }
}
