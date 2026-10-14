//! HOP 3: SQL for `get_invoice`.

use billing_engine::invoice::MAX_LINES;
use chrono::{DateTime, Utc};
use datastore::TenantTx;
use uuid::Uuid;

use crate::InvoiceId;

#[derive(Debug)]
pub(super) struct InvoiceRow {
    pub(super) id: Uuid,
    pub(super) customer_email: String,
    pub(super) currency: String,
    pub(super) status: String,
    pub(super) subtotal_minor: i64,
    pub(super) tax_rate_bps: i32,
    pub(super) tax_minor: i64,
    pub(super) total_minor: i64,
    pub(super) version: i32,
    pub(super) issued_at: DateTime<Utc>,
    pub(super) voided_at: Option<DateTime<Utc>>,
    pub(super) void_reason: Option<String>,
}

#[derive(Debug)]
pub(super) struct LineRow {
    pub(super) description: String,
    pub(super) quantity: i32,
    pub(super) unit_price_minor: i64,
    pub(super) line_total_minor: i64,
}

pub(super) async fn fetch_invoice(
    tx: &mut TenantTx,
    invoice_id: InvoiceId,
) -> Result<Option<InvoiceRow>, sqlx::Error> {
    sqlx::query_as!(
        InvoiceRow,
        r#"
        SELECT id, customer_email, currency, status, subtotal_minor, tax_rate_bps,
               tax_minor, total_minor, version, issued_at, voided_at, void_reason
        FROM billing.invoices
        WHERE tenant_id = $1 AND id = $2
        "#,
        tx.tenant_id().as_uuid(),
        invoice_id.as_uuid(),
    )
    .fetch_optional(tx.conn())
    .await
}

/// Bounded by construction: an invoice never has more than `MAX_LINES` lines.
pub(super) async fn fetch_lines(
    tx: &mut TenantTx,
    invoice_id: InvoiceId,
) -> Result<Vec<LineRow>, sqlx::Error> {
    let max_lines = i64::try_from(MAX_LINES).unwrap_or(i64::MAX);
    sqlx::query_as!(
        LineRow,
        r#"
        SELECT description, quantity, unit_price_minor, line_total_minor
        FROM billing.invoice_lines
        WHERE tenant_id = $1 AND invoice_id = $2
        ORDER BY line_no
        LIMIT $3
        "#,
        tx.tenant_id().as_uuid(),
        invoice_id.as_uuid(),
        max_lines,
    )
    .fetch_all(tx.conn())
    .await
}
