//! HOP 1: HTTP adapter for `void_invoice`.

use api_kit::{ApiError, ApiResult, Json, Path, Precondition, ProblemDetails};
use axum::extract::State;
use axum::response::IntoResponse;
use datastore::DataStore;
use op_context::OpCtx;
use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;

use super::{VoidInvoiceCmd, void_invoice};
use crate::dto::{InvoiceResponse, etag_header};
use crate::{InvoiceId, VoidReason};

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct VoidInvoiceRequest {
    /// Why the invoice is voided (kept for audit).
    #[schema(example = "Issued to the wrong customer")]
    reason: String,
}

/// Void an issued invoice.
#[utoipa::path(
    post,
    path = "/v1/invoices/{invoice_id}/void",
    operation_id = "voidInvoice",
    tag = "billing",
    request_body = VoidInvoiceRequest,
    params(
        ("invoice_id" = Uuid, Path, description = "Invoice id."),
        ("If-Match" = String, Header, description = "The invoice's current ETag."),
    ),
    responses(
        (status = 200, description = "The voided invoice.", body = InvoiceResponse, headers(("ETag" = String))),
        (status = 400, description = "Invalid request.", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 403, description = "Missing `billing.invoice.void`.", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such invoice for this tenant.", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 409, description = "The invoice cannot be voided in its current state.", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 412, description = "Stale ETag.", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 428, description = "If-Match missing.", body = ProblemDetails, content_type = "application/problem+json"),
    ),
    security(("bearer" = [])),
)]
pub(crate) async fn handle(
    ctx: OpCtx,
    State(db): State<DataStore>,
    Path(invoice_id): Path<Uuid>,
    precondition: Precondition,
    Json(request): Json<VoidInvoiceRequest>,
) -> ApiResult<impl IntoResponse> {
    let cmd = VoidInvoiceCmd {
        invoice_id: InvoiceId::from_uuid(invoice_id),
        expected_version: precondition.required_version()?,
        reason: VoidReason::parse(&request.reason)
            .map_err(|reason| ApiError::validation("reason", reason))?,
    };
    let invoice = void_invoice(&ctx, &db, cmd).await?;
    Ok((
        etag_header(invoice.version),
        Json(InvoiceResponse::from(invoice)),
    ))
}
