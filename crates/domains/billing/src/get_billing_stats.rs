//! `GET /v1/billing/stats`: issued/voided totals per currency.
//! Eventually consistent: maintained by the `record_invoice_stats` consumer.

mod db;
pub(crate) mod http;

use datastore::DataStore;
use op_context::OpCtx;

use crate::types::parse_currency;
use crate::{BillingError, CurrencyStats, permissions};

/// HOP 2: read-only workflow.
#[tracing::instrument(
    name = "billing.get_billing_stats",
    skip_all,
    fields(tenant_id = %ctx.tenant_id(), actor_id = %ctx.actor_id())
)]
pub async fn get_billing_stats(
    ctx: &OpCtx,
    db: &DataStore,
) -> Result<Vec<CurrencyStats>, BillingError> {
    ctx.require(permissions::INVOICE_READ)?;

    let mut tx = db.begin_read_only(ctx).await?;
    let rows = db::list(&mut tx).await?;
    tx.commit().await?;

    rows.into_iter()
        .map(|row| {
            Ok(CurrencyStats {
                currency: parse_currency(&row.currency)?,
                issued_count: row.issued_count,
                issued_total_minor: row.issued_total_minor,
                voided_count: row.voided_count,
                voided_total_minor: row.voided_total_minor,
            })
        })
        .collect()
}
