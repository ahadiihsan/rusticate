//! `PUT /v1/billing/settings`: create or update the tenant's billing settings.
//!
//! Optimistic concurrency: `If-None-Match: *` creates, `If-Match: "<version>"`
//! updates. A stale version fails with 412 instead of overwriting.

mod db;
pub(crate) mod http;

use billing_engine::money::{Bps, Currency};
use datastore::DataStore;
use op_context::OpCtx;

use crate::{BillingError, BillingSettings, permissions};

/// What the caller believes about the current settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expectation {
    /// The tenant has no settings yet.
    Create,
    /// The settings are at exactly this version.
    Update { version: i32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfigureBillingCmd {
    pub currency: Currency,
    pub tax_rate: Bps,
    pub expectation: Expectation,
}

/// HOP 2: authorize → transaction → conditional write → commit.
#[tracing::instrument(
    name = "billing.configure_billing",
    skip_all,
    fields(tenant_id = %ctx.tenant_id(), actor_id = %ctx.actor_id())
)]
pub async fn configure_billing(
    ctx: &OpCtx,
    db: &DataStore,
    cmd: ConfigureBillingCmd,
) -> Result<BillingSettings, BillingError> {
    ctx.require(permissions::SETTINGS_MANAGE)?;

    let mut tx = db.begin(ctx).await?;
    let saved = match cmd.expectation {
        Expectation::Create => db::insert(&mut tx, &cmd)
            .await?
            .ok_or(BillingError::SettingsAlreadyExist)?,
        Expectation::Update { version } => db::update(&mut tx, &cmd, version)
            .await?
            .ok_or(BillingError::StaleVersion)?,
    };
    let settings = BillingSettings::try_from(saved)?;
    tx.commit().await?;

    tracing::info!(version = settings.version, "billing settings saved");
    Ok(settings)
}
