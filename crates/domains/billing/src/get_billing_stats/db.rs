//! HOP 3: SQL for `get_billing_stats`.

use datastore::TenantTx;

#[derive(Debug)]
pub(super) struct StatsRow {
    pub(super) currency: String,
    pub(super) issued_count: i64,
    pub(super) issued_total_minor: i64,
    pub(super) voided_count: i64,
    pub(super) voided_total_minor: i64,
}

/// Bounded: one row per currency a tenant has ever invoiced in.
pub(super) async fn list(tx: &mut TenantTx) -> Result<Vec<StatsRow>, sqlx::Error> {
    sqlx::query_as!(
        StatsRow,
        r#"
        SELECT currency, issued_count, issued_total_minor, voided_count, voided_total_minor
        FROM billing.invoice_stats
        WHERE tenant_id = $1
        ORDER BY currency
        LIMIT 64
        "#,
        tx.tenant_id().as_uuid(),
    )
    .fetch_all(tx.conn())
    .await
}
