//! `GET /v1/invoices`: newest-first keyset pagination, optional status filter.

mod db;
pub(crate) mod http;

use api_kit::pagination::{Page, PageRequest};
use billing_engine::invoice::InvoiceStatus;
use datastore::DataStore;
use op_context::OpCtx;

use crate::types::{parse_currency, parse_status};
use crate::{BillingError, CustomerEmail, InvoiceId, InvoiceSummary, permissions};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListInvoicesQuery {
    pub page: PageRequest,
    pub status: Option<InvoiceStatus>,
}

/// HOP 2: read-only workflow.
#[tracing::instrument(
    name = "billing.list_invoices",
    skip_all,
    fields(tenant_id = %ctx.tenant_id(), actor_id = %ctx.actor_id())
)]
pub async fn list_invoices(
    ctx: &OpCtx,
    db: &DataStore,
    query: ListInvoicesQuery,
) -> Result<Page<InvoiceSummary>, BillingError> {
    ctx.require(permissions::INVOICE_READ)?;

    let mut tx = db.begin_read_only(ctx).await?;
    let rows = match query.status {
        None => db::list_all(&mut tx, &query.page).await?,
        Some(status) => db::list_by_status(&mut tx, &query.page, status).await?,
    };
    tx.commit().await?;

    let summaries = rows
        .iter()
        .map(into_summary)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(query
        .page
        .paginate(summaries, |invoice| invoice.id.as_uuid()))
}

fn into_summary(row: &db::SummaryRow) -> Result<InvoiceSummary, BillingError> {
    Ok(InvoiceSummary {
        id: InvoiceId::from_uuid(row.id),
        status: parse_status(&row.status)?,
        customer_email: CustomerEmail::parse(&row.customer_email)
            .map_err(|_| BillingError::Corrupt(format!("invoice {}: customer email", row.id)))?,
        currency: parse_currency(&row.currency)?,
        total_minor: row.total_minor,
        version: row.version,
        issued_at: row.issued_at,
    })
}
