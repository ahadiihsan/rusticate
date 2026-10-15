//! Schema migrations for this product's database (DB-011).
//!
//! Each bounded context owns its schema and migration history. The product
//! applies the platform migrator first, then the migrator of every mounted
//! context. Run as a separate step (Kubernetes Job / pre-deploy hook) with the
//! owner role, never from the serving process.

use std::str::FromStr as _;

use anyhow::Context as _;
use secrecy::{ExposeSecret as _, SecretString};
use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};

/// Every migrator this product needs, in dependency order. Adding a bounded
/// context to the router means adding its migrator here.
pub const MIGRATORS: &[(&str, &Migrator)] = &[
    ("platform", &datastore::MIGRATOR),
    // xtask:migrators
    // rusticate:module(example):start
    ("billing", &billing::MIGRATOR),
    // rusticate:module(example):end
];

pub async fn run(url: &SecretString) -> anyhow::Result<()> {
    let options = PgConnectOptions::from_str(url.expose_secret())?.application_name("api-migrate");
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .context("connecting with the migration role")?;
    for (name, migrator) in MIGRATORS {
        migrator
            .run(&pool)
            .await
            .with_context(|| format!("applying `{name}` migrations"))?;
        tracing::info!(context = name, "migrations applied");
    }
    pool.close().await;
    Ok(())
}
