use api_kit::ApiError;
use datastore::{DataStoreError, TenantTx};
use serde::Serialize;
use serde::de::DeserializeOwned;
use sqlx::types::Json;

use crate::{Fingerprint, IdempotencyKey};

/// Outcome of claiming a key.
#[derive(Debug)]
pub enum Claim<R> {
    /// First use: run the workflow, then call [`Pending::complete`].
    Fresh(Pending),
    /// Seen before with the same payload: return this stored result unchanged.
    Replay(R),
}

/// A claimed key that must be completed in the same transaction.
#[derive(Debug)]
#[must_use = "a claimed idempotency key must be completed with `Pending::complete` before commit"]
pub struct Pending {
    operation: &'static str,
    key: String,
}

#[derive(Debug, thiserror::Error)]
pub enum IdempotencyError {
    #[error("idempotency key was already used with a different request payload")]
    KeyReused,
    #[error("a request with this idempotency key is still in progress")]
    InFlight,
    #[error("stored idempotent response could not be decoded")]
    CorruptRecord(#[source] serde_json::Error),
    #[error(transparent)]
    Store(DataStoreError),
}

impl From<DataStoreError> for IdempotencyError {
    fn from(err: DataStoreError) -> Self {
        match err {
            // The duplicate waited on the first request's uncommitted claim
            // for longer than lock_timeout.
            DataStoreError::LockTimeout => Self::InFlight,
            other => Self::Store(other),
        }
    }
}

impl From<sqlx::Error> for IdempotencyError {
    fn from(err: sqlx::Error) -> Self {
        DataStoreError::from(err).into()
    }
}

impl From<IdempotencyError> for ApiError {
    fn from(err: IdempotencyError) -> Self {
        match err {
            IdempotencyError::KeyReused => Self::unprocessable(
                "IDEMPOTENCY_KEY_REUSED",
                "This Idempotency-Key was already used with a different request payload.",
            ),
            IdempotencyError::InFlight => Self::conflict(
                "IDEMPOTENCY_REQUEST_IN_FLIGHT",
                "A request with this Idempotency-Key is still being processed. Retry later.",
            ),
            IdempotencyError::CorruptRecord(source) => Self::internal(source),
            IdempotencyError::Store(store) => store.into(),
        }
    }
}

/// Claims `key` for `operation` within `tx` (scoped to tx's tenant and actor).
///
/// Keys expire after 24 hours; an expired key is reclaimed as fresh.
pub async fn claim<R: DeserializeOwned>(
    tx: &mut TenantTx,
    operation: &'static str,
    key: &IdempotencyKey,
    fingerprint: &Fingerprint,
) -> Result<Claim<R>, IdempotencyError> {
    let tenant_id = tx.tenant_id().as_uuid();
    let actor_id = tx.actor_id().as_uuid();

    let claimed = sqlx::query_scalar!(
        r#"
        INSERT INTO platform.idempotency_keys
            (tenant_id, actor_id, operation, idempotency_key, fingerprint, expires_at)
        VALUES ($1, $2, $3, $4, $5, now() + interval '24 hours')
        ON CONFLICT (tenant_id, actor_id, operation, idempotency_key) DO UPDATE
            SET fingerprint = EXCLUDED.fingerprint,
                response    = NULL,
                created_at  = now(),
                expires_at  = EXCLUDED.expires_at
            WHERE platform.idempotency_keys.expires_at <= now()
        RETURNING true AS "claimed!"
        "#,
        tenant_id,
        actor_id,
        operation,
        key.as_str(),
        fingerprint.as_bytes(),
    )
    .fetch_optional(tx.conn())
    .await?;

    if claimed.is_some() {
        return Ok(Claim::Fresh(Pending {
            operation,
            key: key.as_str().to_owned(),
        }));
    }

    let existing = sqlx::query!(
        r#"
        SELECT fingerprint, response AS "response: Json<serde_json::Value>"
        FROM platform.idempotency_keys
        WHERE tenant_id = $1 AND actor_id = $2 AND operation = $3 AND idempotency_key = $4
        "#,
        tenant_id,
        actor_id,
        operation,
        key.as_str(),
    )
    .fetch_one(tx.conn())
    .await?;

    if existing.fingerprint != fingerprint.as_bytes() {
        return Err(IdempotencyError::KeyReused);
    }
    match existing.response {
        Some(Json(stored)) => serde_json::from_value(stored)
            .map(Claim::Replay)
            .map_err(IdempotencyError::CorruptRecord),
        // Unreachable when callers complete in the same transaction; treat a
        // missing response as still in progress rather than replaying nothing.
        None => Err(IdempotencyError::InFlight),
    }
}

impl Pending {
    /// Stores the workflow's result for replay. Call before `tx.commit()`.
    pub async fn complete<R: Serialize + Sync>(
        self,
        tx: &mut TenantTx,
        result: &R,
    ) -> Result<(), IdempotencyError> {
        let tenant_id = tx.tenant_id().as_uuid();
        let actor_id = tx.actor_id().as_uuid();
        sqlx::query!(
            r#"
            UPDATE platform.idempotency_keys
            SET response = $5
            WHERE tenant_id = $1 AND actor_id = $2 AND operation = $3 AND idempotency_key = $4
            "#,
            tenant_id,
            actor_id,
            self.operation,
            self.key,
            Json(result) as _,
        )
        .execute(tx.conn())
        .await?;
        Ok(())
    }
}
