use serde::Serialize;
use uuid::Uuid;

use crate::{DataStoreError, TenantTx};

/// A domain event published through the transactional outbox (MSG-001).
///
/// The event type string is bound to the payload type, so a payload can never
/// be published under the wrong name. Event types are versioned and immutable:
/// a breaking payload change is a new type (`...v2`) published alongside the old
/// one until every consumer has migrated (MSG-003).
pub trait DomainEvent: Serialize + Sync {
    /// `<context>.<aggregate>.<past_tense_verb>.v<N>`, e.g. `billing.invoice.issued.v1`.
    const EVENT_TYPE: &'static str;
    /// `<context>.<aggregate>`, e.g. `billing.invoice`.
    const AGGREGATE_TYPE: &'static str;

    fn aggregate_id(&self) -> Uuid;
}

/// Largest serialized event (MSG-003). Events carry identifiers and facts,
/// not documents; a bounded size also guarantees every outbox row is
/// publishable (the broker's message limit is far larger), so one row can
/// never block the relay for every tenant.
pub const MAX_EVENT_BYTES: usize = 64 * 1024;

impl TenantTx {
    /// Records `event` in the outbox, atomically with this transaction's state
    /// changes. Returns the event id (also used as the broker deduplication id).
    ///
    /// Never publish to a broker directly from a workflow (MSG-002).
    pub async fn emit<E: DomainEvent>(&mut self, event: &E) -> Result<Uuid, DataStoreError> {
        let payload = serde_json::to_string(event)?;
        if payload.len() > MAX_EVENT_BYTES {
            return Err(DataStoreError::EventTooLarge {
                event_type: E::EVENT_TYPE,
                bytes: payload.len(),
            });
        }
        let event_id = Uuid::now_v7();
        sqlx::query!(
            r#"
            INSERT INTO platform.outbox_events
                (id, tenant_id, event_type, aggregate_type, aggregate_id, payload,
                 correlation_id, traceparent)
            VALUES ($1, $2, $3, $4, $5, $6::text::jsonb, $7, $8)
            "#,
            event_id,
            self.tenant_id().as_uuid(),
            E::EVENT_TYPE,
            E::AGGREGATE_TYPE,
            event.aggregate_id(),
            payload,
            self.correlation_id().as_uuid(),
            telemetry::current_traceparent(),
        )
        .execute(self.conn())
        .await?;
        Ok(event_id)
    }
}
