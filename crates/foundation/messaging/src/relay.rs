use std::time::Duration;

use datastore::{DataStoreError, SystemStore};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{EventSink, OutboundEvent, SinkError};

/// Advisory lock key held (transaction-scoped) by the active relay.
const RELAY_LOCK_KEY: i64 = 0x6f75_7462_6f78_0001; // "outbox" + 1

#[derive(Debug, Clone)]
pub struct RelayConfig {
    /// Events per transaction. Larger batches amortize round trips; smaller
    /// ones shorten how long the batch transaction stays open.
    pub batch_size: u16,
    /// Sleep when the outbox is empty.
    pub idle_poll: Duration,
    /// Sleep after a failed batch.
    pub error_backoff: Duration,
    /// Upper bound for publishing one batch. The batch transaction is idle
    /// while it waits for broker acks, so this MUST stay below the system
    /// pool's `idle_in_transaction_session_timeout` (15 s by default):
    /// a slow broker then fails the batch cleanly for a retry instead of
    /// Postgres killing the session mid-transaction.
    pub publish_timeout: Duration,
}

impl Default for RelayConfig {
    fn default() -> Self {
        Self {
            batch_size: 500,
            idle_poll: Duration::from_millis(200),
            error_backoff: Duration::from_secs(1),
            publish_timeout: Duration::from_secs(10),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RelayError {
    #[error(transparent)]
    Store(#[from] DataStoreError),
    #[error(transparent)]
    Sink(#[from] SinkError),
    #[error("publishing the batch exceeded {0:?}")]
    PublishTimeout(Duration),
}

impl From<sqlx::Error> for RelayError {
    fn from(err: sqlx::Error) -> Self {
        Self::Store(err.into())
    }
}

/// Moves events from `platform.outbox_events` to an [`EventSink`] (MSG-002).
#[derive(Debug)]
pub struct OutboxRelay<S> {
    store: SystemStore,
    sink: S,
    config: RelayConfig,
}

impl<S: EventSink> OutboxRelay<S> {
    pub fn new(store: SystemStore, sink: S, config: RelayConfig) -> Self {
        Self {
            store,
            sink,
            config,
        }
    }

    /// Relays one batch. Returns how many events were published; `0` when the
    /// outbox is empty or another relay instance holds the lock.
    ///
    /// Publish happens before delete, inside one transaction: a crash after
    /// publish republishes the batch, which the broker (event id) and consumer
    /// inboxes deduplicate. Events are never lost.
    #[tracing::instrument(name = "outbox.relay_batch", skip_all, fields(published))]
    pub async fn relay_batch(&self) -> Result<usize, RelayError> {
        let mut tx = self.store.begin().await?;

        let is_leader = sqlx::query_scalar!(
            r#"SELECT pg_try_advisory_xact_lock($1) AS "acquired!""#,
            RELAY_LOCK_KEY
        )
        .fetch_one(tx.conn())
        .await?;
        if !is_leader {
            tx.commit().await?;
            return Ok(0);
        }

        let events = sqlx::query_as!(
            OutboundEvent,
            r#"
            SELECT id, tenant_id, event_type, aggregate_type, aggregate_id, payload,
                   correlation_id, traceparent, occurred_at
            FROM platform.outbox_events
            ORDER BY id
            LIMIT $1
            "#,
            i64::from(self.config.batch_size),
        )
        .fetch_all(tx.conn())
        .await?;

        if events.is_empty() {
            tx.commit().await?;
            return Ok(0);
        }

        tokio::time::timeout(self.config.publish_timeout, self.sink.publish(&events))
            .await
            .map_err(|_| RelayError::PublishTimeout(self.config.publish_timeout))??;

        let ids: Vec<Uuid> = events.iter().map(|event| event.id).collect();
        sqlx::query!(
            "DELETE FROM platform.outbox_events WHERE id = ANY($1)",
            &ids
        )
        .execute(tx.conn())
        .await?;
        tx.commit().await?;

        tracing::Span::current().record("published", events.len());
        Ok(events.len())
    }

    /// Relays until `shutdown` is cancelled. An in-flight batch always
    /// completes; only the idle sleep is interrupted.
    pub async fn run(self, shutdown: CancellationToken) {
        let full_batch = usize::from(self.config.batch_size);
        while !shutdown.is_cancelled() {
            let pause = match self.relay_batch().await {
                Ok(published) if published >= full_batch => continue,
                Ok(_) => self.config.idle_poll,
                Err(err) => {
                    tracing::error!(error = %err, "outbox relay batch failed");
                    self.config.error_backoff
                }
            };
            tokio::select! {
                () = shutdown.cancelled() => break,
                () = tokio::time::sleep(pause) => {}
            }
        }
        tracing::info!("outbox relay stopped");
    }
}
