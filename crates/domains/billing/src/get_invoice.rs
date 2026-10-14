//! `GET /v1/invoices/{invoice_id}`: one invoice with its lines.

mod db;
pub(crate) mod http;

use datastore::{DataStore, TenantTx};
use op_context::OpCtx;

use crate::types::{parse_currency, parse_status};
use crate::{BillingError, CustomerEmail, InvoiceId, InvoiceLineView, InvoiceView, permissions};

/// HOP 2: read-only workflow. Another tenant's invoice is indistinguishable
/// from a missing one (404), so ids leak nothing.
#[tracing::instrument(
    name = "billing.get_invoice",
    skip_all,
    fields(tenant_id = %ctx.tenant_id(), actor_id = %ctx.actor_id(), invoice_id = %invoice_id)
)]
pub async fn get_invoice(
    ctx: &OpCtx,
    db: &DataStore,
    invoice_id: InvoiceId,
) -> Result<InvoiceView, BillingError> {
    ctx.require(permissions::INVOICE_READ)?;

    let mut tx = db.begin_read_only(ctx).await?;
    let invoice = load_in_tx(&mut tx, invoice_id).await?;
    tx.commit().await?;

    invoice.ok_or(BillingError::InvoiceNotFound)
}

/// Intra-context composition helper (ARCH-006): loads an invoice inside the
/// caller's transaction. Performs no permission check; the calling workflow
/// already did.
pub(crate) async fn load_in_tx(
    tx: &mut TenantTx,
    invoice_id: InvoiceId,
) -> Result<Option<InvoiceView>, BillingError> {
    let Some(row) = db::fetch_invoice(tx, invoice_id).await? else {
        return Ok(None);
    };
    let lines = db::fetch_lines(tx, invoice_id).await?;
    into_view(row, lines).map(Some)
}

fn into_view(row: db::InvoiceRow, lines: Vec<db::LineRow>) -> Result<InvoiceView, BillingError> {
    let corrupt = |what: &str| BillingError::Corrupt(format!("invoice {}: {what}", row.id));
    Ok(InvoiceView {
        id: InvoiceId::from_uuid(row.id),
        status: parse_status(&row.status)?,
        customer_email: CustomerEmail::parse(&row.customer_email)
            .map_err(|_| corrupt("customer email"))?,
        currency: parse_currency(&row.currency)?,
        lines: lines
            .into_iter()
            .map(|line| {
                Ok(InvoiceLineView {
                    description: line.description,
                    quantity: u32::try_from(line.quantity).map_err(|_| corrupt("quantity"))?,
                    unit_price_minor: line.unit_price_minor,
                    line_total_minor: line.line_total_minor,
                })
            })
            .collect::<Result<_, BillingError>>()?,
        subtotal_minor: row.subtotal_minor,
        tax_rate_bps: u16::try_from(row.tax_rate_bps).map_err(|_| corrupt("tax rate"))?,
        tax_minor: row.tax_minor,
        total_minor: row.total_minor,
        version: row.version,
        issued_at: row.issued_at,
        voided_at: row.voided_at,
        void_reason: row.void_reason,
    })
}
