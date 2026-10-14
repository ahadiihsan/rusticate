//! Retention for platform tables (OPS-004). Runs in the worker as `app_system`.
//!
//! Deletes in bounded batches so a backlog never becomes one long transaction.

use std::time::Duration;

use datastore::{DataStoreError, SystemStore};
use tokio_util::sync::CancellationToken;

const BATCH: i64 = 5_000;

/// Inbox rows must outlive every possible redelivery: the stream keeps
/// messages for 7 days, so 14 days of inbox history is safely longer.
const INBOX_RETENTION_DAYS: i32 = 14;

/// Deletes expired idempotency keys and old inbox rows. Returns rows removed.
pub async fn purge_once(store: &SystemStore) -> Result<u64, DataStoreError> {
    let mut tx = store.begin().await?;
    let keys = sqlx::query!(
        r#"
        DELETE FROM platform.idempotency_keys
        WHERE ctid = ANY (ARRAY(
            SELECT ctid FROM platform.idempotency_keys
            WHERE expires_at < now()
            LIMIT $1
        ))
        "#,
        BATCH
    )
    .execute(tx.conn())
    .await?
    .rows_affected();
    let inbox = sqlx::query!(
        r#"
        DELETE FROM platform.inbox_events
        WHERE ctid = ANY (ARRAY(
            SELECT ctid FROM platform.inbox_events
            WHERE processed_at < now() - make_interval(days => $2)
            LIMIT $1
        ))
        "#,
        BATCH,
        INBOX_RETENTION_DAYS
    )
    .execute(tx.conn())
    .await?
    .rows_affected();
    tx.commit().await?;
    Ok(keys.saturating_add(inbox))
}

/// Purges every `interval` until `shutdown`.
pub async fn run(store: SystemStore, interval: Duration, shutdown: CancellationToken) {
    while !shutdown.is_cancelled() {
        match purge_once(&store).await {
            Ok(0) => {}
            Ok(removed) => tracing::info!(removed, "purged expired platform rows"),
            Err(err) => tracing::error!(error = %err, "platform maintenance failed"),
        }
        tokio::select! {
            () = shutdown.cancelled() => break,
            () = tokio::time::sleep(interval) => {}
        }
    }
}
