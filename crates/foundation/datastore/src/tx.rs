use op_context::{ActorId, CorrelationId, OpCtx, TenantId};
use sqlx::{PgConnection, Postgres, Transaction};

use crate::DataStoreError;

/// A transaction with PostgreSQL row-level security bound to one tenant.
///
/// The tenant is set with `set_config(..., is_local => true)`, so it vanishes
/// at COMMIT/ROLLBACK and a pooled connection can never carry it into another
/// request. Dropping a `TenantTx` without [`commit`](Self::commit) rolls back.
#[must_use = "a TenantTx rolls back unless `commit` is awaited"]
pub struct TenantTx {
    tx: Transaction<'static, Postgres>,
    tenant_id: TenantId,
    actor_id: ActorId,
    correlation_id: CorrelationId,
}

impl std::fmt::Debug for TenantTx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TenantTx")
            .field("tenant_id", &self.tenant_id)
            .field("actor_id", &self.actor_id)
            .finish_non_exhaustive()
    }
}

impl TenantTx {
    pub(crate) async fn bind(
        mut tx: Transaction<'static, Postgres>,
        ctx: &OpCtx,
    ) -> Result<Self, DataStoreError> {
        sqlx::query!(
            r#"
            SELECT
                set_config('app.tenant_id', $1, true) AS "tenant_id!",
                set_config('app.actor_id', $2, true) AS "actor_id!"
            "#,
            ctx.tenant_id().to_string(),
            ctx.actor_id().to_string(),
        )
        .fetch_one(&mut *tx)
        .await?;
        Ok(Self {
            tx,
            tenant_id: ctx.tenant_id(),
            actor_id: ctx.actor_id(),
            correlation_id: ctx.correlation_id(),
        })
    }

    pub fn tenant_id(&self) -> TenantId {
        self.tenant_id
    }

    pub fn actor_id(&self) -> ActorId {
        self.actor_id
    }

    pub fn correlation_id(&self) -> CorrelationId {
        self.correlation_id
    }

    /// The connection to pass to `query!(..).fetch_*(tx.conn())`.
    pub fn conn(&mut self) -> &mut PgConnection {
        &mut self.tx
    }

    pub async fn commit(self) -> Result<(), DataStoreError> {
        self.tx.commit().await?;
        Ok(())
    }

    pub async fn rollback(self) -> Result<(), DataStoreError> {
        self.tx.rollback().await?;
        Ok(())
    }
}

/// A cross-tenant transaction on the system store. No tenant is bound; GRANTs
/// limit it to `platform.*` tables.
#[must_use = "a SystemTx rolls back unless `commit` is awaited"]
pub struct SystemTx {
    tx: Transaction<'static, Postgres>,
}

impl std::fmt::Debug for SystemTx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SystemTx").finish_non_exhaustive()
    }
}

impl SystemTx {
    pub(crate) fn new(tx: Transaction<'static, Postgres>) -> Self {
        Self { tx }
    }

    pub fn conn(&mut self) -> &mut PgConnection {
        &mut self.tx
    }

    pub async fn commit(self) -> Result<(), DataStoreError> {
        self.tx.commit().await?;
        Ok(())
    }
}
