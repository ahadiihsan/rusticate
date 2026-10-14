//! `GET /v1/billing/settings`: the tenant's billing settings and their ETag.

mod db;
pub(crate) mod http;

use datastore::DataStore;
use op_context::OpCtx;

use crate::{BillingError, BillingSettings, permissions};

/// HOP 2: read-only workflow. `None` when billing is not configured yet.
#[tracing::instrument(
    name = "billing.get_billing_settings",
    skip_all,
    fields(tenant_id = %ctx.tenant_id(), actor_id = %ctx.actor_id())
)]
pub async fn get_billing_settings(
    ctx: &OpCtx,
    db: &DataStore,
) -> Result<Option<BillingSettings>, BillingError> {
    ctx.require(permissions::SETTINGS_READ)?;

    let mut tx = db.begin_read_only(ctx).await?;
    let row = db::load(&mut tx).await?;
    tx.commit().await?;

    row.map(BillingSettings::try_from).transpose()
}
