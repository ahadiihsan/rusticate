//! HOP 3: SQL for `list_invoices`.
//!
//! PERF-005: no `($n IS NULL OR col = $n)` predicates. Postgres caches a
//! generic plan for prepared statements, and a generic plan cannot drop the
//! OR, so it stops using the index. Instead: one query per filter shape, and
//! the "first page" cursor is the maximum UUID, so `id < $2` is always a
//! sargable range on the primary key / status index.

use api_kit::pagination::PageRequest;
use billing_engine::invoice::InvoiceStatus;
use chrono::{DateTime, Utc};
use datastore::TenantTx;
use uuid::Uuid;

#[derive(Debug)]
pub(super) struct SummaryRow {
    pub(super) id: Uuid,
    pub(super) customer_email: String,
    pub(super) currency: String,
    pub(super) status: String,
    pub(super) total_minor: i64,
    pub(super) version: i32,
    pub(super) issued_at: DateTime<Utc>,
}

pub(super) async fn list_all(
    tx: &mut TenantTx,
    page: &PageRequest,
) -> Result<Vec<SummaryRow>, sqlx::Error> {
    sqlx::query_as!(
        SummaryRow,
        r#"
        SELECT id, customer_email, currency, status, total_minor, version, issued_at
        FROM billing.invoices
        WHERE tenant_id = $1 AND id < $2
        ORDER BY id DESC
        LIMIT $3
        "#,
        tx.tenant_id().as_uuid(),
        page.after().unwrap_or(Uuid::max()),
        page.fetch_limit(),
    )
    .fetch_all(tx.conn())
    .await
}

pub(super) async fn list_by_status(
    tx: &mut TenantTx,
    page: &PageRequest,
    status: InvoiceStatus,
) -> Result<Vec<SummaryRow>, sqlx::Error> {
    sqlx::query_as!(
        SummaryRow,
        r#"
        SELECT id, customer_email, currency, status, total_minor, version, issued_at
        FROM billing.invoices
        WHERE tenant_id = $1 AND status = $2 AND id < $3
        ORDER BY id DESC
        LIMIT $4
        "#,
        tx.tenant_id().as_uuid(),
        status.as_str(),
        page.after().unwrap_or(Uuid::max()),
        page.fetch_limit(),
    )
    .fetch_all(tx.conn())
    .await
}
