//! HOP 3: SQL for `record_invoice_stats`.

use billing_engine::money::Currency;
use datastore::TenantTx;

#[derive(Debug)]
pub(super) struct Delta {
    pub(super) currency: Currency,
    pub(super) issued_count: i64,
    pub(super) issued_total_minor: i64,
    pub(super) voided_count: i64,
    pub(super) voided_total_minor: i64,
}

/// Atomic upsert-increment: commutative, so event order does not matter.
pub(super) async fn apply(tx: &mut TenantTx, delta: &Delta) -> Result<(), sqlx::Error> {
    sqlx::query!(
        r#"
        INSERT INTO billing.invoice_stats
            (tenant_id, currency, issued_count, issued_total_minor, voided_count, voided_total_minor)
        VALUES ($1, $2, $3, $4, $5, $6)
        ON CONFLICT (tenant_id, currency) DO UPDATE SET
            issued_count       = billing.invoice_stats.issued_count       + EXCLUDED.issued_count,
            issued_total_minor = billing.invoice_stats.issued_total_minor + EXCLUDED.issued_total_minor,
            voided_count       = billing.invoice_stats.voided_count       + EXCLUDED.voided_count,
            voided_total_minor = billing.invoice_stats.voided_total_minor + EXCLUDED.voided_total_minor,
            updated_at         = now()
        "#,
        tx.tenant_id().as_uuid(),
        delta.currency.code(),
        delta.issued_count,
        delta.issued_total_minor,
        delta.voided_count,
        delta.voided_total_minor,
    )
    .execute(tx.conn())
    .await?;
    Ok(())
}
