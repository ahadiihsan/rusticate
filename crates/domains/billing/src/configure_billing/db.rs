//! HOP 3: SQL for `configure_billing`. No business branching.

use datastore::TenantTx;

use super::ConfigureBillingCmd;
use crate::types::SettingsRow;

/// Inserts settings unless the tenant already has some (then `None`).
pub(super) async fn insert(
    tx: &mut TenantTx,
    cmd: &ConfigureBillingCmd,
) -> Result<Option<SettingsRow>, sqlx::Error> {
    sqlx::query_as!(
        SettingsRow,
        r#"
        INSERT INTO billing.settings (tenant_id, currency, tax_rate_bps, updated_by)
        VALUES ($1, $2, $3, $4)
        ON CONFLICT (tenant_id) DO NOTHING
        RETURNING currency, tax_rate_bps, version
        "#,
        tx.tenant_id().as_uuid(),
        cmd.currency.code(),
        i32::from(cmd.tax_rate.get()),
        tx.actor_id().as_uuid(),
    )
    .fetch_optional(tx.conn())
    .await
}

/// Updates settings only at `expected_version` (else `None`).
pub(super) async fn update(
    tx: &mut TenantTx,
    cmd: &ConfigureBillingCmd,
    expected_version: i32,
) -> Result<Option<SettingsRow>, sqlx::Error> {
    sqlx::query_as!(
        SettingsRow,
        r#"
        UPDATE billing.settings
        SET currency = $2,
            tax_rate_bps = $3,
            updated_by = $4,
            updated_at = now(),
            version = version + 1
        WHERE tenant_id = $1 AND version = $5
        RETURNING currency, tax_rate_bps, version
        "#,
        tx.tenant_id().as_uuid(),
        cmd.currency.code(),
        i32::from(cmd.tax_rate.get()),
        tx.actor_id().as_uuid(),
        expected_version,
    )
    .fetch_optional(tx.conn())
    .await
}
