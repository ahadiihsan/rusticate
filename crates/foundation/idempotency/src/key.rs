use api_kit::ApiError;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;

/// Request header carrying the client-chosen key.
pub const HEADER: &str = "idempotency-key";

const MAX_LEN: usize = 255;

/// A validated `Idempotency-Key` header: 1..=255 visible ASCII characters.
///
/// Taking `IdempotencyKey` as a handler argument makes the header mandatory
/// for that route (missing → 400 `IDEMPOTENCY_KEY_REQUIRED`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdempotencyKey(String);

impl IdempotencyKey {
    pub fn parse(raw: &str) -> Result<Self, ApiError> {
        let valid =
            !raw.is_empty() && raw.len() <= MAX_LEN && raw.bytes().all(|b| b.is_ascii_graphic());
        if valid {
            Ok(Self(raw.to_owned()))
        } else {
            Err(ApiError::validation(
                "Idempotency-Key",
                format!("must be 1 to {MAX_LEN} visible ASCII characters"),
            ))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<S: Send + Sync> FromRequestParts<S> for IdempotencyKey {
    type Rejection = ApiError;

    fn from_request_parts(
        parts: &mut Parts,
        _state: &S,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
        let result = match parts.headers.get(HEADER) {
            None => Err(ApiError::BadRequest {
                code: "IDEMPOTENCY_KEY_REQUIRED",
                detail: "This operation requires an Idempotency-Key header.".to_owned(),
                violations: Vec::new(),
            }),
            Some(value) => value
                .to_str()
                .map_err(|_| ApiError::validation("Idempotency-Key", "must be visible ASCII"))
                .and_then(Self::parse),
        };
        std::future::ready(result)
    }
}

#[cfg(test)]
mod tests {
    use axum::http::Request;

    use super::*;

    async fn extract(header: Option<&str>) -> Result<IdempotencyKey, ApiError> {
        let mut builder = Request::builder();
        if let Some(value) = header {
            builder = builder.header(HEADER, value);
        }
        let (mut parts, ()) = builder.body(()).unwrap().into_parts();
        IdempotencyKey::from_request_parts(&mut parts, &()).await
    }

    #[tokio::test]
    async fn missing_header_is_required_error() {
        let err = extract(None).await.unwrap_err();

        assert_eq!(err.code(), "IDEMPOTENCY_KEY_REQUIRED");
    }

    #[tokio::test]
    async fn accepts_uuid_like_keys_and_rejects_oversized_or_blank() {
        let long = "k".repeat(MAX_LEN + 1);

        assert!(
            extract(Some("8f14e45f-ceea-467f-a1d5-1c2e8b6b3c9a"))
                .await
                .is_ok()
        );
        assert_eq!(
            extract(Some(&long)).await.unwrap_err().code(),
            "VALIDATION_FAILED"
        );
        assert_eq!(
            extract(Some("has space")).await.unwrap_err().code(),
            "VALIDATION_FAILED"
        );
    }
}
