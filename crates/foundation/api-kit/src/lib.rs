//! HTTP contract primitives shared by every service (SOP API-001..API-008).
//!
//! * [`ProblemDetails`] / [`ApiError`]: every error response is RFC 9457
//!   `application/problem+json` with a stable `error_code` and a `trace_id`
//!   that matches the distributed trace.
//! * [`Json`], [`Path`], [`Query`]: drop-in replacements for the axum
//!   extractors whose rejections are also `problem+json`. The axum originals are
//!   banned by `clippy.toml`.
//! * [`RequestScope`]: per-request facts (request id, path) available to error
//!   rendering without threading them through every function.
//! * [`pagination`]: opaque-cursor keyset pagination.
//! * [`Precondition`]: `ETag` / `If-Match` optimistic concurrency.

mod error;
mod extract;
pub mod pagination;
mod precondition;
mod problem;
mod scope;

pub use error::{ApiError, ApiResult};
pub use extract::{Json, Path, Query};
pub use precondition::{Precondition, etag, precondition_failed, precondition_required};
pub use problem::{FieldViolation, PROBLEM_JSON, ProblemDetails};
pub use scope::RequestScope;
