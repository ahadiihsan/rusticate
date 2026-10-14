//! HOP 1: HTTP adapter for `get_billing_stats`.

use api_kit::{ApiResult, Json, ProblemDetails};
use axum::extract::State;
use datastore::DataStore;
use op_context::OpCtx;
use serde::Serialize;
use utoipa::ToSchema;

use super::get_billing_stats;
use crate::CurrencyStats;

/// Invoice statistics, per currency.
#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct BillingStatsResponse {
    currencies: Vec<CurrencyStatsResponse>,
}

#[derive(Debug, Serialize, ToSchema)]
pub(crate) struct CurrencyStatsResponse {
    #[schema(example = "IDR")]
    currency: String,
    issued_count: i64,
    issued_total_minor: i64,
    voided_count: i64,
    voided_total_minor: i64,
}

impl From<CurrencyStats> for CurrencyStatsResponse {
    fn from(stats: CurrencyStats) -> Self {
        Self {
            currency: stats.currency.code().to_owned(),
            issued_count: stats.issued_count,
            issued_total_minor: stats.issued_total_minor,
            voided_count: stats.voided_count,
            voided_total_minor: stats.voided_total_minor,
        }
    }
}

/// Invoice statistics (eventually consistent, typically within seconds).
#[utoipa::path(
    get,
    path = "/v1/billing/stats",
    operation_id = "getBillingStats",
    tag = "billing",
    responses(
        (status = 200, description = "Statistics per currency.", body = BillingStatsResponse),
        (status = 403, description = "Missing `billing.invoice.read`.", body = ProblemDetails, content_type = "application/problem+json"),
    ),
    security(("bearer" = [])),
)]
pub(crate) async fn handle(
    ctx: OpCtx,
    State(db): State<DataStore>,
) -> ApiResult<Json<BillingStatsResponse>> {
    let stats = get_billing_stats(&ctx, &db).await?;
    Ok(Json(BillingStatsResponse {
        currencies: stats.into_iter().map(CurrencyStatsResponse::from).collect(),
    }))
}
