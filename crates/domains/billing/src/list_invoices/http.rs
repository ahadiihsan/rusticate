//! HOP 1: HTTP adapter for `list_invoices`.

use api_kit::pagination::{PageParams, PageRequest};
use api_kit::{ApiError, ApiResult, Json, ProblemDetails, Query};
use axum::extract::State;
use billing_engine::invoice::InvoiceStatus;
use datastore::DataStore;
use op_context::OpCtx;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use super::{ListInvoicesQuery, list_invoices};
use crate::dto::InvoiceSummaryResponse;

#[derive(Debug, Deserialize, IntoParams)]
#[serde(deny_unknown_fields)]
#[into_params(parameter_in = Query)]
pub(crate) struct ListInvoicesParams {
    /// Page size, 1..=100 (default 50).
    limit: Option<u32>,
    /// Opaque cursor from the previous page's `next_cursor`.
    cursor: Option<String>,
    /// Only invoices in this status: `issued`, `paid` or `void`.
    status: Option<String>,
}

impl TryFrom<ListInvoicesParams> for ListInvoicesQuery {
    type Error = ApiError;

    fn try_from(params: ListInvoicesParams) -> Result<Self, Self::Error> {
        let status = params
            .status
            .as_deref()
            .map(str::parse::<InvoiceStatus>)
            .transpose()
            .map_err(|_| ApiError::validation("status", "must be one of: issued, paid, void"))?;
        let page = PageRequest::try_from(PageParams {
            limit: params.limit,
            cursor: params.cursor,
        })?;
        Ok(Self { page, status })
    }
}

/// One page of invoices, newest first.
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct InvoicePageResponse {
    items: Vec<InvoiceSummaryResponse>,
    /// Pass as `cursor` for the next page. Absent on the last page.
    #[serde(skip_serializing_if = "Option::is_none")]
    next_cursor: Option<String>,
}

/// List invoices.
#[utoipa::path(
    get,
    path = "/v1/invoices",
    operation_id = "listInvoices",
    tag = "billing",
    params(ListInvoicesParams),
    responses(
        (status = 200, description = "One page of invoices.", body = InvoicePageResponse),
        (status = 400, description = "Invalid filter or cursor.", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 403, description = "Missing `billing.invoice.read`.", body = ProblemDetails, content_type = "application/problem+json"),
    ),
    security(("bearer" = [])),
)]
pub(crate) async fn handle(
    ctx: OpCtx,
    State(db): State<DataStore>,
    Query(params): Query<ListInvoicesParams>,
) -> ApiResult<Json<InvoicePageResponse>> {
    let query = ListInvoicesQuery::try_from(params)?;
    let page = list_invoices(&ctx, &db, query).await?;
    Ok(Json(InvoicePageResponse {
        items: page
            .items
            .into_iter()
            .map(InvoiceSummaryResponse::from)
            .collect(),
        next_cursor: page.next_cursor,
    }))
}
