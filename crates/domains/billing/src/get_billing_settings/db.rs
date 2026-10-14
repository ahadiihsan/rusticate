//! HOP 3: SQL for `get_billing_settings`.

use datastore::TenantTx;

use crate::types::SettingsRow;

pub(super) async fn load(tx: &mut TenantTx) -> Result<Option<SettingsRow>, sqlx::Error> {
    sqlx::query_as!(
        SettingsRow,
        r#"
        SELECT currency, tax_rate_bps, version
        FROM billing.settings
        WHERE tenant_id = $1
        "#,
        tx.tenant_id().as_uuid(),
    )
    .fetch_optional(tx.conn())
    .await
}
