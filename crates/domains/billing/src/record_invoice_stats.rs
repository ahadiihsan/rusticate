//! Event-driven feature: keeps `billing.invoice_stats` up to date from invoice
//! events. Same three hops; hop 1 is an event handler instead of an HTTP
//! handler, and the runtime provides the (deduplicated, tenant-bound)
//! transaction.

mod db;
mod handler;

use billing_engine::money::Currency;
use datastore::TenantTx;
use op_context::OpCtx;

pub use handler::RecordInvoiceStats;

use crate::{BillingError, permissions};

/// The effect of one invoice event on the statistics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatsChange {
    Issued {
        currency: Currency,
        total_minor: i64,
    },
    Voided {
        currency: Currency,
        total_minor: i64,
    },
}

/// HOP 2 (consumer form): works inside the transaction the consumer runtime
/// opened, so the inbox record and this effect commit together.
#[tracing::instrument(name = "billing.record_invoice_stats", skip_all, fields(tenant_id = %ctx.tenant_id()))]
pub async fn record_invoice_stats(
    ctx: &OpCtx,
    tx: &mut TenantTx,
    change: StatsChange,
) -> Result<(), BillingError> {
    ctx.require(permissions::STATS_RECORD)?;

    let delta = match change {
        StatsChange::Issued {
            currency,
            total_minor,
        } => db::Delta {
            currency,
            issued_count: 1,
            issued_total_minor: total_minor,
            voided_count: 0,
            voided_total_minor: 0,
        },
        StatsChange::Voided {
            currency,
            total_minor,
        } => db::Delta {
            currency,
            issued_count: 0,
            issued_total_minor: 0,
            voided_count: 1,
            voided_total_minor: total_minor,
        },
    };
    db::apply(tx, &delta).await?;
    Ok(())
}
