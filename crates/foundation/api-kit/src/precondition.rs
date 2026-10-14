//! HTTP optimistic concurrency (SOP API-007, RFC 9110 §13).
//!
//! Mutable resources carry an integer `version` column, exposed as a strong
//! `ETag: "<version>"`. Updates must send `If-Match: "<version>"`; a stale
//! version fails with 412 instead of silently overwriting someone else's
//! change ("lost update"). Creation of a singleton uses `If-None-Match: *`.

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::{HeaderValue, StatusCode, header};

use crate::ApiError;

/// The conditional-request headers of a write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Precondition {
    /// No conditional header was sent.
    None,
    /// `If-Match: "<version>"`: apply only if the current version matches.
    IfMatch(i32),
    /// `If-None-Match: *`: apply only if the resource does not exist yet.
    IfNoneMatchAny,
}

impl Precondition {
    /// The version an update must match; 428 when the client sent none.
    pub fn required_version(self) -> Result<i32, ApiError> {
        match self {
            Self::IfMatch(version) => Ok(version),
            Self::None | Self::IfNoneMatchAny => Err(precondition_required(
                "Send If-Match with the resource's current ETag.",
            )),
        }
    }
}

/// `ETag` value for `version`.
pub fn etag(version: i32) -> HeaderValue {
    HeaderValue::from(version)
        .to_str()
        .ok()
        .and_then(|digits| HeaderValue::from_str(&format!("\"{digits}\"")).ok())
        .unwrap_or_else(|| HeaderValue::from_static("\"0\""))
}

/// 428: a conditional header is mandatory for this write.
pub fn precondition_required(detail: impl Into<String>) -> ApiError {
    ApiError::Rejected {
        status: StatusCode::PRECONDITION_REQUIRED,
        code: "PRECONDITION_REQUIRED",
        detail: detail.into(),
    }
}

/// 412: the precondition did not hold (stale `If-Match`, or the resource
/// already exists under `If-None-Match: *`).
pub fn precondition_failed(detail: impl Into<String>) -> ApiError {
    ApiError::Rejected {
        status: StatusCode::PRECONDITION_FAILED,
        code: "PRECONDITION_FAILED",
        detail: detail.into(),
    }
}

impl<S: Send + Sync> FromRequestParts<S> for Precondition {
    type Rejection = ApiError;

    fn from_request_parts(
        parts: &mut Parts,
        _state: &S,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
        std::future::ready(parse(parts))
    }
}

fn parse(parts: &Parts) -> Result<Precondition, ApiError> {
    if let Some(value) = parts.headers.get(header::IF_MATCH) {
        let raw = value.to_str().unwrap_or_default().trim();
        // Strong comparison only: weak validators (W/"...") never match writes.
        let version = raw
            .strip_prefix('"')
            .and_then(|rest| rest.strip_suffix('"'))
            .and_then(|digits| digits.parse::<i32>().ok())
            .ok_or_else(|| {
                ApiError::validation("If-Match", "must be a single strong ETag such as \"3\"")
            })?;
        return Ok(Precondition::IfMatch(version));
    }
    if let Some(value) = parts.headers.get(header::IF_NONE_MATCH) {
        if value.to_str().is_ok_and(|v| v.trim() == "*") {
            return Ok(Precondition::IfNoneMatchAny);
        }
        return Err(ApiError::validation(
            "If-None-Match",
            "only `*` is supported on writes",
        ));
    }
    Ok(Precondition::None)
}

#[cfg(test)]
mod tests {
    use axum::http::Request;

    use super::*;

    fn parts(name: &str, value: &str) -> Parts {
        Request::builder()
            .header(name, value)
            .body(())
            .unwrap()
            .into_parts()
            .0
    }

    #[test]
    fn parses_strong_etags_and_star() {
        assert_eq!(
            parse(&parts("if-match", "\"7\"")).unwrap(),
            Precondition::IfMatch(7)
        );
        assert_eq!(
            parse(&parts("if-none-match", "*")).unwrap(),
            Precondition::IfNoneMatchAny
        );
        assert_eq!(
            parse(&Request::new(()).into_parts().0).unwrap(),
            Precondition::None
        );
    }

    #[test]
    fn rejects_weak_or_malformed_etags() {
        assert!(parse(&parts("if-match", "W/\"7\"")).is_err());
        assert!(parse(&parts("if-match", "7")).is_err());
        assert!(parse(&parts("if-none-match", "\"7\"")).is_err());
    }

    #[test]
    fn missing_if_match_is_precondition_required() {
        let err = Precondition::None.required_version().unwrap_err();

        assert_eq!(err.status(), StatusCode::PRECONDITION_REQUIRED);
    }

    #[test]
    fn etag_is_quoted_version() {
        assert_eq!(etag(12), "\"12\"");
    }
}
