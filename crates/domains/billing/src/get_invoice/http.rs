//! HOP 1: HTTP adapter for `get_invoice`.

use api_kit::{ApiResult, Json, Path, ProblemDetails};
use axum::extract::State;
use axum::response::IntoResponse;
use datastore::DataStore;
use op_context::OpCtx;
use uuid::Uuid;

use super::get_invoice;
use crate::InvoiceId;
use crate::dto::{InvoiceResponse, etag_header};

/// One invoice.
#[utoipa::path(
    get,
    path = "/v1/invoices/{invoice_id}",
    operation_id = "getInvoice",
    tag = "billing",
    params(("invoice_id" = Uuid, Path, description = "Invoice id.")),
    responses(
        (status = 200, description = "The invoice.", body = InvoiceResponse, headers(("ETag" = String))),
        (status = 403, description = "Missing `billing.invoice.read`.", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "No such invoice for this tenant.", body = ProblemDetails, content_type = "application/problem+json"),
    ),
    security(("bearer" = [])),
)]
pub(crate) async fn handle(
    ctx: OpCtx,
    State(db): State<DataStore>,
    Path(invoice_id): Path<Uuid>,
) -> ApiResult<impl IntoResponse> {
    let invoice = get_invoice(&ctx, &db, InvoiceId::from_uuid(invoice_id)).await?;
    Ok((
        etag_header(invoice.version),
        Json(InvoiceResponse::from(invoice)),
    ))
}
