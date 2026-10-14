//! HOP 3: SQL for `void_invoice`.

use chrono::{DateTime, Utc};
use datastore::TenantTx;

use super::VoidInvoiceCmd;
use crate::InvoiceId;

#[derive(Debug)]
pub(super) struct VoidTarget {
    pub(super) status: String,
    pub(super) version: i32,
    pub(super) currency: String,
    pub(super) total_minor: i64,
}

pub(super) async fn fetch_target(
    tx: &mut TenantTx,
    invoice_id: InvoiceId,
) -> Result<Option<VoidTarget>, sqlx::Error> {
    sqlx::query_as!(
        VoidTarget,
        r#"
        SELECT status, version, currency, total_minor
        FROM billing.invoices
        WHERE tenant_id = $1 AND id = $2
        "#,
        tx.tenant_id().as_uuid(),
        invoice_id.as_uuid(),
    )
    .fetch_optional(tx.conn())
    .await
}

/// Applies the transition only at the expected version (else `None`).
pub(super) async fn apply_void(
    tx: &mut TenantTx,
    cmd: &VoidInvoiceCmd,
    new_status: &str,
) -> Result<Option<DateTime<Utc>>, sqlx::Error> {
    sqlx::query_scalar!(
        r#"
        UPDATE billing.invoices
        SET status = $3,
            voided_at = now(),
            void_reason = $4,
            version = version + 1
        WHERE tenant_id = $1 AND id = $2 AND version = $5
        RETURNING voided_at AS "voided_at!"
        "#,
        tx.tenant_id().as_uuid(),
        cmd.invoice_id.as_uuid(),
        new_status,
        cmd.reason.as_str(),
        cmd.expected_version,
    )
    .fetch_optional(tx.conn())
    .await
}
