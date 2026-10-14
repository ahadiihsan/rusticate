use api_kit::ApiError;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;

use crate::{AuthzError, OpCtx};

/// `OpCtx` as a handler argument. The IAM middleware inserts it into the
/// request extensions after verifying credentials; extensions cannot be set by
/// clients. Handlers that take `OpCtx` are therefore authenticated endpoints:
/// a missing context is a 401.
impl<S: Send + Sync> FromRequestParts<S> for OpCtx {
    type Rejection = ApiError;

    fn from_request_parts(
        parts: &mut Parts,
        _state: &S,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
        std::future::ready(
            parts
                .extensions
                .get::<Self>()
                .cloned()
                .ok_or(ApiError::Unauthenticated),
        )
    }
}

impl From<AuthzError> for ApiError {
    fn from(err: AuthzError) -> Self {
        Self::Forbidden {
            detail: err.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use axum::http::{Request, StatusCode};

    use super::*;

    #[tokio::test]
    async fn missing_context_is_unauthenticated() {
        let (mut parts, ()) = Request::new(()).into_parts();

        let rejection = OpCtx::from_request_parts(&mut parts, &())
            .await
            .unwrap_err();

        assert_eq!(rejection.status(), StatusCode::UNAUTHORIZED);
    }
}
