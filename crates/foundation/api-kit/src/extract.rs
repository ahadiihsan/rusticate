//! Problem-aware replacements for `axum::{Json, extract::Path, extract::Query}`.
//!
//! The axum types render rejections as `text/plain`, which would break the
//! "every error is problem+json" contract (API-001). The originals are banned
//! workspace-wide by `clippy.toml`; this module is the one sanctioned wrapper.
#![expect(
    clippy::disallowed_types,
    reason = "this module is the sanctioned wrapper around the banned axum extractors"
)]

use axum::extract::rejection::{JsonRejection, PathRejection, QueryRejection};
use axum::extract::{FromRequest, FromRequestParts, Request};
use axum::http::StatusCode;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::ApiError;

/// JSON request body / response body.
#[derive(Debug, Clone, Copy, Default)]
pub struct Json<T>(pub T);

impl<T, S> FromRequest<S> for Json<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        let axum::Json(value) = axum::Json::<T>::from_request(req, state).await.map_err(
            |rejection: JsonRejection| {
                reject(
                    rejection.status(),
                    rejection.body_text(),
                    "INVALID_REQUEST_BODY",
                )
            },
        )?;
        Ok(Self(value))
    }
}

impl<T: Serialize> IntoResponse for Json<T> {
    fn into_response(self) -> Response {
        axum::Json(self.0).into_response()
    }
}

/// Typed path parameters.
#[derive(Debug, Clone, Copy, Default)]
pub struct Path<T>(pub T);

impl<T, S> FromRequestParts<S> for Path<T>
where
    T: DeserializeOwned + Send,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let axum::extract::Path(value) = axum::extract::Path::<T>::from_request_parts(parts, state)
            .await
            .map_err(|rejection: PathRejection| {
                reject(
                    rejection.status(),
                    rejection.body_text(),
                    "INVALID_PATH_PARAMETER",
                )
            })?;
        Ok(Self(value))
    }
}

/// Typed query string.
#[derive(Debug, Clone, Copy, Default)]
pub struct Query<T>(pub T);

impl<T, S> FromRequestParts<S> for Query<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let axum::extract::Query(value) =
            axum::extract::Query::<T>::from_request_parts(parts, state)
                .await
                .map_err(|rejection: QueryRejection| {
                    reject(
                        rejection.status(),
                        rejection.body_text(),
                        "INVALID_QUERY_PARAMETER",
                    )
                })?;
        Ok(Self(value))
    }
}

/// Maps an axum rejection to a problem.
///
/// Shape errors (axum's 422 "data error") are reported as 400: in this API, 422
/// is reserved for well-formed requests that violate a domain rule (API-002).
fn reject(status: StatusCode, detail: String, shape_code: &'static str) -> ApiError {
    match status {
        StatusCode::BAD_REQUEST | StatusCode::UNPROCESSABLE_ENTITY => ApiError::BadRequest {
            code: shape_code,
            detail,
            violations: Vec::new(),
        },
        StatusCode::PAYLOAD_TOO_LARGE => ApiError::Rejected {
            status,
            code: "PAYLOAD_TOO_LARGE",
            detail,
        },
        StatusCode::UNSUPPORTED_MEDIA_TYPE => ApiError::Rejected {
            status,
            code: "UNSUPPORTED_MEDIA_TYPE",
            detail,
        },
        _ if status.is_server_error() => ApiError::internal(std::io::Error::other(detail)),
        _ => ApiError::Rejected {
            status,
            code: "REQUEST_REJECTED",
            detail,
        },
    }
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::body::Body;
    use axum::routing::post;
    use http_body_util::BodyExt as _;
    use tower::ServiceExt as _;

    use super::*;

    #[derive(Debug, serde::Deserialize)]
    struct Payload {
        #[expect(dead_code, reason = "only deserialization is exercised")]
        quantity: u32,
    }

    async fn handler(Json(_): Json<Payload>) -> StatusCode {
        StatusCode::NO_CONTENT
    }

    async fn post_body(
        body: &'static str,
        content_type: &'static str,
    ) -> (StatusCode, serde_json::Value) {
        let app = Router::new().route("/", post(handler));
        let request = axum::http::Request::post("/")
            .header("content-type", content_type)
            .body(Body::from(body))
            .unwrap();
        let response = app.oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
        )
    }

    #[tokio::test]
    async fn malformed_json_is_problem_400() {
        let (status, body) = post_body("{not json", "application/json").await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error_code"], "INVALID_REQUEST_BODY");
    }

    #[tokio::test]
    async fn wrong_shape_is_problem_400_not_422() {
        let (status, body) = post_body(r#"{"quantity":"many"}"#, "application/json").await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error_code"], "INVALID_REQUEST_BODY");
    }

    async fn get(uri: &str) -> (StatusCode, serde_json::Value) {
        #[derive(Debug, serde::Deserialize)]
        struct Page {
            #[expect(dead_code, reason = "only deserialization is exercised")]
            limit: u32,
        }
        async fn by_id(Path(_): Path<uuid::Uuid>) -> StatusCode {
            StatusCode::NO_CONTENT
        }
        async fn paged(Query(_): Query<Page>) -> StatusCode {
            StatusCode::NO_CONTENT
        }
        let app = Router::new()
            .route("/things/{id}", axum::routing::get(by_id))
            .route("/things", axum::routing::get(paged));
        let request = axum::http::Request::get(uri).body(Body::empty()).unwrap();
        let response = app.oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
        )
    }

    #[tokio::test]
    async fn bad_path_and_query_parameters_are_problem_400() {
        let (path_status, path_body) = get("/things/not-a-uuid").await;
        let (query_status, query_body) = get("/things?limit=lots").await;
        let (ok_status, _) = get(&format!("/things/{}", uuid::Uuid::nil())).await;

        assert_eq!(path_status, StatusCode::BAD_REQUEST);
        assert_eq!(path_body["error_code"], "INVALID_PATH_PARAMETER");
        assert_eq!(query_status, StatusCode::BAD_REQUEST);
        assert_eq!(query_body["error_code"], "INVALID_QUERY_PARAMETER");
        assert_eq!(ok_status, StatusCode::NO_CONTENT);
    }

    #[test]
    fn server_side_rejections_become_internal_errors() {
        let err = reject(
            StatusCode::INTERNAL_SERVER_ERROR,
            "missing extension".to_owned(),
            "X",
        );

        assert_eq!(err.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(
            reject(StatusCode::PAYLOAD_TOO_LARGE, String::new(), "X").code(),
            "PAYLOAD_TOO_LARGE"
        );
    }

    #[tokio::test]
    async fn missing_content_type_is_problem_415() {
        let (status, body) = post_body(r#"{"quantity":1}"#, "text/plain").await;

        assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
        assert_eq!(body["error_code"], "UNSUPPORTED_MEDIA_TYPE");
    }
}
