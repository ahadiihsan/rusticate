//! JetStream adapter. Thin by design: every decision lives in the
//! broker-independent relay/consumer code above.

use std::num::NonZeroUsize;
use std::time::Duration;

use async_nats::jetstream::consumer::{AckPolicy, pull};
use async_nats::jetstream::stream::{self, Stream};
use async_nats::jetstream::{AckKind, Context, Message};
use datastore::DataStore;
use futures::StreamExt as _;
use tokio_util::sync::CancellationToken;
use tracing::Instrument as _;

use crate::{EventHandler, EventSink, InboundEvent, OutboundEvent, Outcome, SinkError, process};

/// Where events live in JetStream: one stream capturing `<prefix>.>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Topology {
    pub stream: String,
    /// Events are published to `<subject_prefix>.<event_type>`.
    pub subject_prefix: String,
}

impl Default for Topology {
    fn default() -> Self {
        Self {
            stream: "EVENTS".to_owned(),
            subject_prefix: "events".to_owned(),
        }
    }
}

impl Topology {
    fn subject(&self, event_type: &str) -> String {
        format!("{}.{event_type}", self.subject_prefix)
    }
}

/// Redelivery attempts before JetStream gives up on a message.
const MAX_DELIVER: i64 = 10;

#[derive(Debug, thiserror::Error)]
#[error("JetStream operation failed")]
pub struct NatsError(#[source] pub Box<dyn std::error::Error + Send + Sync>);

fn nats_err(err: impl std::error::Error + Send + Sync + 'static) -> NatsError {
    NatsError(Box::new(err))
}

/// Creates (or binds to) the events stream. The duplicate window must exceed
/// the longest plausible relay retry so republished batches are deduplicated.
pub async fn ensure_stream(
    js: &Context,
    topology: &Topology,
    replicas: usize,
) -> Result<Stream, NatsError> {
    js.get_or_create_stream(stream::Config {
        name: topology.stream.clone(),
        subjects: vec![format!("{}.>", topology.subject_prefix)],
        duplicate_window: Duration::from_mins(2),
        max_age: Duration::from_hours(7 * 24),
        storage: stream::StorageType::File,
        num_replicas: replicas,
        ..stream::Config::default()
    })
    .await
    .map_err(nats_err)
}

/// Publishes relayed events to JetStream.
#[derive(Debug, Clone)]
pub struct JetStreamSink {
    js: Context,
    topology: Topology,
}

impl JetStreamSink {
    pub fn new(js: Context, topology: Topology) -> Self {
        Self { js, topology }
    }
}

fn nats_headers(event: &OutboundEvent) -> async_nats::HeaderMap {
    let mut headers = async_nats::HeaderMap::new();
    for (name, value) in &event.header_map() {
        if let Ok(value) = value.to_str() {
            headers.insert(name.as_str(), value);
        }
    }
    headers
}

impl EventSink for JetStreamSink {
    /// Pipelined: sends the whole batch, then awaits every ack.
    async fn publish(&self, events: &[OutboundEvent]) -> Result<(), SinkError> {
        let to_sink = |err: Box<dyn std::error::Error + Send + Sync>| SinkError(err);
        let mut acks = Vec::with_capacity(events.len());
        for event in events {
            let payload = serde_json::to_vec(&event.payload).map_err(|e| to_sink(Box::new(e)))?;
            let ack = self
                .js
                .publish_with_headers(
                    self.topology.subject(&event.event_type),
                    nats_headers(event),
                    payload.into(),
                )
                .await
                .map_err(|e| to_sink(Box::new(e)))?;
            acks.push(ack);
        }
        for ack in acks {
            ack.await.map_err(|e| to_sink(Box::new(e)))?;
        }
        Ok(())
    }
}

/// Consumer loop for one handler. Several worker replicas may run it: the
/// durable consumer load-balances messages between them.
pub async fn run_consumer<H: EventHandler>(
    stream: &Stream,
    topology: &Topology,
    store: DataStore,
    handler: H,
    concurrency: NonZeroUsize,
    shutdown: CancellationToken,
) -> Result<(), NatsError> {
    if !is_valid_consumer_name(H::CONSUMER) {
        return Err(nats_err(std::io::Error::other(format!(
            "invalid consumer name `{}`: use [a-z0-9_-]",
            H::CONSUMER
        ))));
    }
    let consumer = stream
        .get_or_create_consumer(
            H::CONSUMER,
            pull::Config {
                durable_name: Some(H::CONSUMER.to_owned()),
                filter_subjects: H::EVENT_TYPES
                    .iter()
                    .map(|event_type| topology.subject(event_type))
                    .collect(),
                ack_policy: AckPolicy::Explicit,
                ack_wait: Duration::from_secs(30),
                max_deliver: MAX_DELIVER,
                ..pull::Config::default()
            },
        )
        .await
        .map_err(nats_err)?;

    let messages = consumer.messages().await.map_err(nats_err)?;
    messages
        .take_until(shutdown.cancelled_owned())
        .for_each_concurrent(concurrency.get(), |delivery| {
            let store = &store;
            let handler = &handler;
            async move {
                match delivery {
                    Ok(message) => handle_delivery(store, handler, message).await,
                    Err(err) => {
                        tracing::warn!(error = %err, consumer = H::CONSUMER, "delivery error");
                    }
                }
            }
        })
        .await;
    tracing::info!(consumer = H::CONSUMER, "consumer stopped");
    Ok(())
}

async fn handle_delivery<H: EventHandler>(store: &DataStore, handler: &H, message: Message) {
    let headers = message
        .headers
        .as_ref()
        .map(to_http_headers)
        .unwrap_or_default();
    let event = match InboundEvent::from_parts(&headers, &message.payload) {
        Ok(event) => event,
        Err(err) => {
            tracing::error!(error = %err, consumer = H::CONSUMER, "terminating malformed message");
            ack(&message, AckKind::Term).await;
            return;
        }
    };

    let span = tracing::info_span!("event.consume", consumer = H::CONSUMER);
    if let Some(traceparent) = &event.traceparent {
        telemetry::set_parent_from_traceparent(&span, traceparent);
    }
    let result = process(store, handler, &event).instrument(span).await;

    let kind = match result {
        Ok(Outcome::Processed | Outcome::Duplicate) => AckKind::Ack,
        Err(err) if err.is_transient() => {
            let attempt = message.info().map_or(1, |info| info.delivered);
            if attempt >= MAX_DELIVER {
                // Last delivery: terminate explicitly so the dead-letter
                // advisory fires (MSG-007) instead of a silent drop.
                tracing::error!(error = %err, attempt, event_id = %event.id, consumer = H::CONSUMER,
                    "retries exhausted; terminating (dead-lettered)");
                AckKind::Term
            } else {
                tracing::warn!(error = %err, attempt, event_id = %event.id, "transient failure; will retry");
                AckKind::Nak(Some(backoff(attempt)))
            }
        }
        Err(err) => {
            tracing::error!(error = %err, event_id = %event.id, "permanent failure; terminating");
            AckKind::Term
        }
    };
    ack(&message, kind).await;
}

fn is_valid_consumer_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}

/// Exponential backoff: 1s, 2s, 4s, ... capped at 5 minutes.
fn backoff(attempt: i64) -> Duration {
    let exponent = u32::try_from(attempt.saturating_sub(1).clamp(0, 9)).unwrap_or(9);
    Duration::from_secs(2_u64.saturating_pow(exponent)).min(Duration::from_secs(300))
}

async fn ack(message: &Message, kind: AckKind) {
    if let Err(err) = message.ack_with(kind).await {
        tracing::warn!(error = %err, "failed to acknowledge message; it will be redelivered");
    }
}

fn to_http_headers(headers: &async_nats::HeaderMap) -> http::HeaderMap {
    headers
        .iter()
        .filter_map(|(name, values)| {
            let name = http::HeaderName::from_bytes(AsRef::<[u8]>::as_ref(name)).ok()?;
            let value = http::HeaderValue::from_str(values.first()?.as_str()).ok()?;
            Some((name, value))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consumer_names_must_be_jetstream_safe() {
        assert!(is_valid_consumer_name("billing-record-invoice-stats"));
        assert!(!is_valid_consumer_name("billing.record_invoice_stats"));
        assert!(!is_valid_consumer_name(""));
    }

    #[test]
    fn backoff_grows_exponentially_and_is_capped() {
        assert_eq!(backoff(1), Duration::from_secs(1));
        assert_eq!(backoff(4), Duration::from_secs(8));
        assert_eq!(backoff(50), Duration::from_secs(300));
    }
}
