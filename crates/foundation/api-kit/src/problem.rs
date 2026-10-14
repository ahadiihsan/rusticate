use serde::Serialize;
use utoipa::ToSchema;

/// Media type of every error response (RFC 9457 §6.1).
pub const PROBLEM_JSON: &str = "application/problem+json";

/// RFC 9457 Problem Details, extended with stable machine-readable members.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ProblemDetails {
    /// URI reference identifying the problem type: `/problems/{error-code}`.
    #[serde(rename = "type")]
    #[schema(example = "/problems/validation-failed")]
    pub problem_type: String,
    /// Short summary of the problem type (the HTTP reason phrase).
    #[schema(example = "Bad Request")]
    pub title: String,
    /// HTTP status code.
    #[schema(example = 400)]
    pub status: u16,
    /// Explanation specific to this occurrence. Never contains internal details.
    pub detail: String,
    /// URI reference of this occurrence: the request path.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "/v1/invoices")]
    pub instance: Option<String>,
    /// Stable machine-readable code. Clients branch on this, never on `detail`.
    #[schema(example = "VALIDATION_FAILED")]
    pub error_code: String,
    /// W3C trace id of the request, for support correlation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    /// Request id (also echoed in the `x-request-id` response header).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    /// Field-level violations, for validation failures.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<FieldViolation>,
}

/// One invalid input field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct FieldViolation {
    /// JSON path of the offending field, e.g. `lines[2].quantity`.
    pub field: String,
    /// Why the value was rejected.
    pub reason: String,
}

impl FieldViolation {
    pub fn new(field: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            reason: reason.into(),
        }
    }
}

/// `VALIDATION_FAILED` → `/problems/validation-failed`.
pub(crate) fn problem_type_uri(error_code: &str) -> String {
    let slug: String = error_code
        .chars()
        .map(|c| {
            if c == '_' {
                '-'
            } else {
                c.to_ascii_lowercase()
            }
        })
        .collect();
    format!("/problems/{slug}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn problem_type_is_kebab_case_of_error_code() {
        assert_eq!(
            problem_type_uri("IDEMPOTENCY_KEY_REUSED"),
            "/problems/idempotency-key-reused"
        );
    }
}
