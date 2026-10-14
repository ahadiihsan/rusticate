use std::error::Error as StdError;

use datastore::{DataStore, DataStoreError, TenantTx};
use op_context::{OpCtx, Permission, mint};

use crate::InboundEvent;

/// What happened to a delivered event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Effects committed.
    Processed,
    /// Already processed by this consumer (redelivery); nothing done.
    Duplicate,
}

/// Why a handler failed; decides between retry and dead-lettering.
#[derive(Debug, thiserror::Error)]
pub enum HandlerError {
    /// Retrying may succeed (database contention, timeouts).
    #[error("transient handler failure")]
    Transient(#[source] Box<dyn StdError + Send + Sync>),
    /// Retrying cannot succeed (undecodable payload, violated invariant).
    /// The message is terminated and must be investigated.
    #[error("permanent handler failure")]
    Permanent(#[source] Box<dyn StdError + Send + Sync>),
}

impl HandlerError {
    pub fn permanent(source: impl StdError + Send + Sync + 'static) -> Self {
        Self::Permanent(Box::new(source))
    }

    pub fn is_transient(&self) -> bool {
        matches!(self, Self::Transient(_))
    }
}

impl From<DataStoreError> for HandlerError {
    fn from(err: DataStoreError) -> Self {
        if err.is_transient() {
            Self::Transient(Box::new(err))
        } else {
            Self::Permanent(Box::new(err))
        }
    }
}

impl From<serde_json::Error> for HandlerError {
    fn from(err: serde_json::Error) -> Self {
        Self::permanent(err)
    }
}

/// A consumer of domain events (MSG-004). Lives in the domain crate that
/// owns the reaction, next to the workflow it calls.
pub trait EventHandler: Send + Sync {
    /// Durable consumer name, unique and stable: `<context>-<handler>`, only
    /// `[a-z0-9_-]` (JetStream rejects `.`, `*`, `>` and whitespace). Changing
    /// it creates a new consumer that replays the stream from the start.
    const CONSUMER: &'static str;
    /// Event types this handler subscribes to, e.g. `billing.invoice.issued.v1`.
    const EVENT_TYPES: &'static [&'static str];
    /// Exactly the permissions the handler's system context needs.
    const PERMISSIONS: &'static [Permission];

    /// Applies the event's effects inside `tx` (already bound to the event's
    /// tenant, already deduplicated). Must not commit.
    fn handle(
        &self,
        ctx: &OpCtx,
        tx: &mut TenantTx,
        event: &InboundEvent,
    ) -> impl Future<Output = Result<(), HandlerError>> + Send;
}

/// Processes one delivery: inbox dedup + effects in one tenant transaction.
#[tracing::instrument(
    name = "event.process",
    skip_all,
    fields(
        consumer = H::CONSUMER,
        event_id = %event.id,
        event_type = %event.event_type,
        tenant_id = %event.tenant_id,
        correlation_id = %event.correlation_id,
    )
)]
pub async fn process<H: EventHandler>(
    store: &DataStore,
    handler: &H,
    event: &InboundEvent,
) -> Result<Outcome, HandlerError> {
    let ctx = mint::system(event.tenant_id, event.correlation_id, H::PERMISSIONS);
    let mut tx = store.begin(&ctx).await?;
    if !tx.record_inbox(H::CONSUMER, event.id).await? {
        // Dropping the transaction rolls back; nothing was written.
        return Ok(Outcome::Duplicate);
    }
    handler.handle(&ctx, &mut tx, event).await?;
    tx.commit().await?;
    Ok(Outcome::Processed)
}
