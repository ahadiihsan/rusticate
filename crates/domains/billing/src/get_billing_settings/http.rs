//! HOP 1: HTTP adapter for `get_billing_settings`.

use api_kit::{ApiError, ApiResult, Json, ProblemDetails};
use axum::extract::State;
use axum::response::IntoResponse;
use datastore::DataStore;
use op_context::OpCtx;

use super::get_billing_settings;
use crate::dto::{BillingSettingsResponse, etag_header};

/// The tenant's billing settings.
#[utoipa::path(
    get,
    path = "/v1/billing/settings",
    operation_id = "getBillingSettings",
    tag = "billing",
    responses(
        (status = 200, description = "Current settings.", body = BillingSettingsResponse, headers(("ETag" = String))),
        (status = 403, description = "Missing `billing.settings.read`.", body = ProblemDetails, content_type = "application/problem+json"),
        (status = 404, description = "Billing is not configured.", body = ProblemDetails, content_type = "application/problem+json"),
    ),
    security(("bearer" = [])),
)]
pub(crate) async fn handle(
    ctx: OpCtx,
    State(db): State<DataStore>,
) -> ApiResult<impl IntoResponse> {
    let settings = get_billing_settings(&ctx, &db)
        .await?
        .ok_or_else(|| ApiError::not_found("billing settings"))?;
    Ok((
        etag_header(settings.version),
        Json(BillingSettingsResponse::from(settings)),
    ))
}
