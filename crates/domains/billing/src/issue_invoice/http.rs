//! HOP 1: HTTP adapter for `issue_invoice`. Parses the request into a
//! validated command, calls the workflow, renders the response. No business
//! logic, no SQL.

use api_kit::{ApiError, ApiResult, FieldViolation, Json, ProblemDetails};
use axum::extract::State;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::IntoResponse;
use billing_engine::invoice::{MAX_LINES, Quantity};
use billing_engine::money::MAX_MINOR;
use datastore::DataStore;
use idempotency::IdempotencyKey;
use op_context::OpCtx;
use serde::Deserialize;
use utoipa::ToSchema;

use super::{IssueInvoiceCmd, NewLine, issue_invoice};
use crate::dto::{InvoiceResponse, etag_header};
use crate::{CustomerEmail, LineDescription};

/// An invoice to issue. Currency and tax rate come from the tenant's settings.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct IssueInvoiceRequest {
    #[schema(example = "finance@customer.example")]
    customer_email: String,
    /// 1 to 200 lines.
    lines: Vec<IssueInvoiceLine>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct IssueInvoiceLine {
    #[schema(example = "Consulting, March")]
    description: String,
    /// 1 to 1,000,000.
    #[schema(example = 2)]
    quantity: u32,
    /// Unit price in minor units of the tenant currency (e.g. sen for IDR).
    #[schema(example = 15_000_000)]
    unit_price_minor: i64,
}

/// Parse, don't validate: the command can only be built from valid input,
/// and every violation is reported in one response.
impl TryFrom<IssueInvoiceRequest> for IssueInvoiceCmd {
    type Error = ApiError;

    fn try_from(request: IssueInvoiceRequest) -> Result<Self, Self::Error> {
        let mut violations = Vec::new();

        let customer_email = CustomerEmail::parse(&request.customer_email)
            .map_err(|reason| violations.push(FieldViolation::new("customer_email", reason)))
            .ok();

        if request.lines.is_empty() || request.lines.len() > MAX_LINES {
            violations.push(FieldViolation::new(
                "lines",
                format!("must contain between 1 and {MAX_LINES} lines"),
            ));
        }

        let mut lines = Vec::with_capacity(request.lines.len());
        for (index, line) in request.lines.into_iter().enumerate() {
            let description = LineDescription::parse(&line.description)
                .map_err(|reason| {
                    violations.push(FieldViolation::new(
                        format!("lines[{index}].description"),
                        reason,
                    ));
                })
                .ok();
            let quantity = Quantity::new(line.quantity)
                .map_err(|err| {
                    violations.push(FieldViolation::new(
                        format!("lines[{index}].quantity"),
                        err.to_string(),
                    ));
                })
                .ok();
            if !(0..=MAX_MINOR).contains(&line.unit_price_minor) {
                violations.push(FieldViolation::new(
                    format!("lines[{index}].unit_price_minor"),
                    format!("must be between 0 and {MAX_MINOR}"),
                ));
            }
            if let (Some(description), Some(quantity)) = (description, quantity) {
                lines.push(NewLine {
                    description,
                    quantity,
                    unit_price_minor: line.unit_price_minor,
                });
            }
        }

        match customer_email {
            Some(customer_email) if violations.is_empty() => Ok(Self {
                customer_email,
                lines,
            }),
            _ => Err(ApiError::validation_many(violations)),
        }
    }
}

/// Issue an invoice.
///
/// Idempotent: retrying with the same `Idempotency-Key` and body returns the
/// original invoice; a different body with the same key is rejected.
#[utoipa::path(
    post,
    path = "/v1/invoices",
    operation_id = "issueInvoice",
    tag = "billing",
    request_body = IssueInvoiceRequest,
    params(
        ("Idempotency-Key" = String, Header, description = "Client-generated key (e.g. a UUID), unique per intended invoice."),
    ),
    responses(
        (status = 201, description = "Invoice issued (or replayed).", body = InvoiceResponse,
            headers(("Location" = String), ("ETag" = String))),
        (status = 400, description = "Invalid request or missing Idempotency-Key.", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 403, description = "Missing `billing.invoice.issue`.", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "A request with this key is in flight.", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 422, description = "Billing not configured, a business rule failed, or the key was reused with another body.", body = ProblemDetails, content_type = "application/problem+json"),
    ),
    security(("bearer" = [])),
)]
pub(crate) async fn handle(
    ctx: OpCtx,
    State(db): State<DataStore>,
    key: IdempotencyKey,
    Json(request): Json<IssueInvoiceRequest>,
) -> ApiResult<impl IntoResponse> {
    let cmd = IssueInvoiceCmd::try_from(request)?;
    let invoice = issue_invoice(&ctx, &db, &key, cmd).await?;

    let location = HeaderValue::from_str(&format!("/v1/invoices/{}", invoice.id))
        .map_err(ApiError::internal)?;
    Ok((
        StatusCode::CREATED,
        [(header::LOCATION, location)],
        etag_header(invoice.version),
        Json(InvoiceResponse::from(invoice)),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(description: &str, quantity: u32, unit_price_minor: i64) -> IssueInvoiceLine {
        IssueInvoiceLine {
            description: description.to_owned(),
            quantity,
            unit_price_minor,
        }
    }

    #[test]
    fn valid_request_becomes_a_command() {
        let request = IssueInvoiceRequest {
            customer_email: "ap@acme.example".to_owned(),
            lines: vec![line(" Widget ", 3, 1_000)],
        };

        let cmd = IssueInvoiceCmd::try_from(request).unwrap();

        assert_eq!(cmd.lines[0].description.as_str(), "Widget");
        assert_eq!(cmd.lines[0].quantity.get(), 3);
    }

    #[test]
    fn every_violation_is_reported_with_its_json_path() {
        let request = IssueInvoiceRequest {
            customer_email: "not-an-email".to_owned(),
            lines: vec![line("ok", 1, 1), line("", 0, -5)],
        };

        let Err(ApiError::BadRequest { violations, .. }) = IssueInvoiceCmd::try_from(request)
        else {
            panic!("expected validation failure");
        };
        let fields: Vec<_> = violations.iter().map(|v| v.field.as_str()).collect();
        assert_eq!(
            fields,
            [
                "customer_email",
                "lines[1].description",
                "lines[1].quantity",
                "lines[1].unit_price_minor"
            ]
        );
    }

    #[test]
    fn empty_invoice_is_rejected() {
        let request = IssueInvoiceRequest {
            customer_email: "ap@acme.example".to_owned(),
            lines: Vec::new(),
        };

        assert!(IssueInvoiceCmd::try_from(request).is_err());
    }
}
