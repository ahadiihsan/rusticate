use std::error::Error as StdError;

use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};

use crate::problem::{FieldViolation, PROBLEM_JSON, ProblemDetails, problem_type_uri};
use crate::scope::RequestScope;

pub type ApiResult<T> = Result<T, ApiError>;

/// The single error type every HTTP handler returns (SOP API-001).
///
/// Domain crates define their own error enums and convert into this one at the
/// HTTP boundary (`impl From<DomainError> for ApiError`). Infrastructure
/// failures are never rendered verbatim: `Internal` is logged with its full
/// source chain and the client only sees the trace id.
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    /// 400: the request is malformed or a field is invalid.
    #[error("{detail}")]
    BadRequest {
        code: &'static str,
        detail: String,
        violations: Vec<FieldViolation>,
    },
    /// 401: no or invalid credentials.
    #[error("authentication required")]
    Unauthenticated,
    /// 403: authenticated, but lacks a permission.
    #[error("{detail}")]
    Forbidden { detail: String },
    /// 404: the resource does not exist *for this tenant* (also used to hide
    /// existence across tenants).
    #[error("{resource} not found")]
    NotFound { resource: &'static str },
    /// 409: conflicts with the current state (stale version, duplicate, in-flight).
    #[error("{detail}")]
    Conflict { code: &'static str, detail: String },
    /// 422: well-formed, but violates a domain rule.
    #[error("{detail}")]
    UnprocessableEntity { code: &'static str, detail: String },
    /// Framework-level rejection with a specific status (413, 415, ...).
    #[error("{detail}")]
    Rejected {
        status: StatusCode,
        code: &'static str,
        detail: String,
    },
    /// 503: transient failure; the client should retry after `retry_after_secs`.
    #[error("{detail}")]
    ServiceUnavailable {
        code: &'static str,
        detail: String,
        retry_after_secs: u32,
    },
    /// 500: unexpected failure. The source is logged, never returned.
    #[error("internal error")]
    Internal(#[source] Box<dyn StdError + Send + Sync>),
}

impl ApiError {
    /// 400 `VALIDATION_FAILED` for a single field.
    pub fn validation(field: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::validation_many(vec![FieldViolation::new(field, reason)])
    }

    /// 400 `VALIDATION_FAILED` for several fields.
    pub fn validation_many(violations: Vec<FieldViolation>) -> Self {
        Self::BadRequest {
            code: "VALIDATION_FAILED",
            detail: "One or more fields are invalid.".to_owned(),
            violations,
        }
    }

    pub fn not_found(resource: &'static str) -> Self {
        Self::NotFound { resource }
    }

    pub fn conflict(code: &'static str, detail: impl Into<String>) -> Self {
        Self::Conflict {
            code,
            detail: detail.into(),
        }
    }

    pub fn unprocessable(code: &'static str, detail: impl Into<String>) -> Self {
        Self::UnprocessableEntity {
            code,
            detail: detail.into(),
        }
    }

    pub fn unavailable(code: &'static str, detail: impl Into<String>) -> Self {
        Self::ServiceUnavailable {
            code,
            detail: detail.into(),
            retry_after_secs: 1,
        }
    }

    pub fn internal(source: impl StdError + Send + Sync + 'static) -> Self {
        Self::Internal(Box::new(source))
    }

    pub fn status(&self) -> StatusCode {
        match self {
            Self::BadRequest { .. } => StatusCode::BAD_REQUEST,
            Self::Unauthenticated => StatusCode::UNAUTHORIZED,
            Self::Forbidden { .. } => StatusCode::FORBIDDEN,
            Self::NotFound { .. } => StatusCode::NOT_FOUND,
            Self::Conflict { .. } => StatusCode::CONFLICT,
            Self::UnprocessableEntity { .. } => StatusCode::UNPROCESSABLE_ENTITY,
            Self::Rejected { status, .. } => *status,
            Self::ServiceUnavailable { .. } => StatusCode::SERVICE_UNAVAILABLE,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// The stable `error_code` clients branch on.
    pub fn code(&self) -> &'static str {
        match self {
            Self::BadRequest { code, .. }
            | Self::Conflict { code, .. }
            | Self::UnprocessableEntity { code, .. }
            | Self::Rejected { code, .. }
            | Self::ServiceUnavailable { code, .. } => code,
            Self::Unauthenticated => "UNAUTHENTICATED",
            Self::Forbidden { .. } => "FORBIDDEN",
            Self::NotFound { .. } => "RESOURCE_NOT_FOUND",
            Self::Internal(_) => "INTERNAL_ERROR",
        }
    }

    fn log(&self) {
        match self {
            Self::Internal(source) => {
                tracing::error!(error = %DisplayChain(source.as_ref()), "internal error");
            }
            Self::ServiceUnavailable { code, detail, .. } => {
                tracing::warn!(error_code = code, detail, "transient failure");
            }
            other => {
                tracing::debug!(error_code = other.code(), detail = %other, "request rejected");
            }
        }
    }

    fn into_problem(self) -> (StatusCode, ProblemDetails, Option<u32>) {
        let status = self.status();
        let error_code = self.code();
        let retry_after = match &self {
            Self::ServiceUnavailable {
                retry_after_secs, ..
            } => Some(*retry_after_secs),
            _ => None,
        };
        let (detail, errors) = match self {
            Self::BadRequest {
                detail, violations, ..
            } => (detail, violations),
            Self::Internal(_) => (
                "An unexpected error occurred. Quote the trace_id when contacting support."
                    .to_owned(),
                Vec::new(),
            ),
            other => (other.to_string(), Vec::new()),
        };
        let scope = RequestScope::current();
        let problem = ProblemDetails {
            problem_type: problem_type_uri(error_code),
            title: status.canonical_reason().unwrap_or("Error").to_owned(),
            status: status.as_u16(),
            detail,
            instance: scope.as_ref().map(|s| s.path.clone()),
            error_code: error_code.to_owned(),
            trace_id: telemetry::current_trace_id(),
            request_id: scope.map(|s| s.request_id.to_string()),
            errors,
        };
        (status, problem, retry_after)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        self.log();
        let is_unauthenticated = matches!(self, Self::Unauthenticated);
        let (status, problem, retry_after) = self.into_problem();

        let body = match serde_json::to_vec(&problem) {
            Ok(body) => body,
            Err(err) => {
                tracing::error!(error = %err, "failed to serialize problem details");
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        };
        let mut response = (status, body).into_response();
        let headers = response.headers_mut();
        headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(PROBLEM_JSON));
        if let Some(seconds) = retry_after {
            headers.insert(header::RETRY_AFTER, HeaderValue::from(seconds));
        }
        if is_unauthenticated {
            headers.insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
        }
        response
    }
}

/// Renders an error with its full `source()` chain: `a: b: c`.
struct DisplayChain<'a>(&'a (dyn StdError + 'static));

impl std::fmt::Display for DisplayChain<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)?;
        let mut source = self.0.source();
        while let Some(err) = source {
            write!(f, ": {err}")?;
            source = err.source();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use http_body_util::BodyExt as _;

    use super::*;

    async fn render(error: ApiError) -> (StatusCode, axum::http::HeaderMap, serde_json::Value) {
        let response = error.into_response();
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, headers, serde_json::from_slice(&bytes).unwrap())
    }

    #[tokio::test]
    async fn validation_error_renders_problem_json_with_violations() {
        let (status, headers, body) = render(ApiError::validation(
            "lines[0].quantity",
            "must be positive",
        ))
        .await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(headers[header::CONTENT_TYPE], PROBLEM_JSON);
        assert_eq!(body["type"], "/problems/validation-failed");
        assert_eq!(body["error_code"], "VALIDATION_FAILED");
        assert_eq!(body["errors"][0]["field"], "lines[0].quantity");
    }

    #[tokio::test]
    async fn internal_error_never_leaks_its_source() {
        let source = std::io::Error::other("password=hunter2 at db.internal:5432");

        let (status, _, body) = render(ApiError::internal(source)).await;

        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        let text = body.to_string();
        assert!(!text.contains("hunter2"));
        assert!(!text.contains("db.internal"));
    }

    #[tokio::test]
    async fn unavailable_sets_retry_after() {
        let (status, headers, _) = render(ApiError::unavailable("DATABASE_BUSY", "retry")).await;

        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(headers[header::RETRY_AFTER], "1");
    }

    #[tokio::test]
    async fn unauthenticated_sets_www_authenticate() {
        let (status, headers, _) = render(ApiError::Unauthenticated).await;

        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(headers[header::WWW_AUTHENTICATE], "Bearer");
    }

    #[tokio::test]
    async fn request_scope_fills_instance_and_request_id() {
        let scope = RequestScope {
            request_id: uuid::Uuid::nil(),
            path: "/v1/things".to_owned(),
        };

        let (_, _, body) = scope.run(render(ApiError::not_found("thing"))).await;

        assert_eq!(body["instance"], "/v1/things");
        assert_eq!(body["request_id"], uuid::Uuid::nil().to_string());
    }
}
