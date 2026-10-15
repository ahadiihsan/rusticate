use std::str::FromStr as _;

use op_context::OpCtx;
use secrecy::ExposeSecret as _;
use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};

use crate::posture::{Posture, verify};
use crate::{DataStoreConfig, DataStoreError, SystemTx, TenantTx};

async fn connect_pool(
    cfg: &DataStoreConfig,
    application_name: &str,
) -> Result<PgPool, DataStoreError> {
    let connect = PgConnectOptions::from_str(cfg.url.expose_secret())?
        .application_name(application_name)
        .options(cfg.session_options());
    let pool = PgPoolOptions::new()
        .max_connections(cfg.max_connections)
        .min_connections(cfg.min_connections)
        .acquire_timeout(cfg.acquire_timeout())
        // PERF-002: no liveness ping on every acquire (saves one round trip per
        // transaction). Broken connections surface as errors and are discarded.
        .test_before_acquire(false)
        .connect_with(connect)
        .await?;
    Ok(pool)
}

/// The application's database handle. Clone freely (it is a pool handle).
///
/// There is intentionally no way to obtain the raw pool: every statement runs
/// inside a [`TenantTx`] bound to an [`OpCtx`].
#[derive(Clone)]
pub struct DataStore {
    pool: PgPool,
}

impl std::fmt::Debug for DataStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DataStore").finish_non_exhaustive()
    }
}

impl DataStore {
    /// Connects as the RLS-bound application role and verifies its posture.
    pub async fn connect(
        cfg: &DataStoreConfig,
        application_name: &str,
    ) -> Result<Self, DataStoreError> {
        Self::from_pool(connect_pool(cfg, application_name).await?).await
    }

    /// Wraps an existing pool after verifying it is RLS-bound (used by test-kit).
    pub async fn from_pool(pool: PgPool) -> Result<Self, DataStoreError> {
        verify(&pool, Posture::RlsEnforced).await?;
        Ok(Self { pool })
    }

    /// Opens a read-write transaction bound to `ctx`'s tenant.
    pub async fn begin(&self, ctx: &OpCtx) -> Result<TenantTx, DataStoreError> {
        let tx = self.pool.begin().await?;
        TenantTx::bind(tx, ctx).await
    }

    /// Opens a read-only transaction bound to `ctx`'s tenant (DB-006). Use for
    /// every query-only workflow: Postgres rejects accidental writes, and the
    /// transaction can later be routed to a replica.
    pub async fn begin_read_only(&self, ctx: &OpCtx) -> Result<TenantTx, DataStoreError> {
        let tx = self.pool.begin_with("BEGIN READ ONLY").await?;
        TenantTx::bind(tx, ctx).await
    }

    /// Readiness probe.
    pub async fn ping(&self) -> Result<(), DataStoreError> {
        sqlx::query!("SELECT 1 AS one")
            .fetch_one(&self.pool)
            .await?;
        Ok(())
    }

    /// Waits for in-flight connections to be returned, then closes the pool.
    pub async fn close(&self) {
        self.pool.close().await;
    }
}

/// Cross-tenant store for the outbox relay and maintenance jobs (DB-002).
///
/// Connects as `app_system` (BYPASSRLS, GRANTs limited to `platform.*`).
/// Usage is restricted to the crates listed under `[capabilities.system_store]`
/// in `architecture.toml`.
#[derive(Clone)]
pub struct SystemStore {
    pool: PgPool,
}

impl std::fmt::Debug for SystemStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SystemStore").finish_non_exhaustive()
    }
}

impl SystemStore {
    pub async fn connect(
        cfg: &DataStoreConfig,
        application_name: &str,
    ) -> Result<Self, DataStoreError> {
        Self::from_pool(connect_pool(cfg, application_name).await?).await
    }

    pub async fn from_pool(pool: PgPool) -> Result<Self, DataStoreError> {
        verify(&pool, Posture::System).await?;
        Ok(Self { pool })
    }

    pub async fn begin(&self) -> Result<SystemTx, DataStoreError> {
        Ok(SystemTx::new(self.pool.begin().await?))
    }

    pub async fn ping(&self) -> Result<(), DataStoreError> {
        sqlx::query!("SELECT 1 AS one")
            .fetch_one(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn close(&self) {
        self.pool.close().await;
    }
}

impl api_kit::ReadinessCheck for DataStore {
    fn name(&self) -> &'static str {
        "database"
    }

    fn check(&self) -> api_kit::CheckFuture<'_> {
        Box::pin(async move { self.ping().await.map_err(|err| err.to_string()) })
    }
}

impl api_kit::ReadinessCheck for SystemStore {
    fn name(&self) -> &'static str {
        "database_system"
    }

    fn check(&self) -> api_kit::CheckFuture<'_> {
        Box::pin(async move { self.ping().await.map_err(|err| err.to_string()) })
    }
}
