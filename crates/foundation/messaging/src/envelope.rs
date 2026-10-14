use chrono::{DateTime, Utc};
use http::HeaderMap;
use op_context::{CorrelationId, TenantId};
use serde::de::DeserializeOwned;
use uuid::Uuid;

/// Broker message header names (the wire contract between relay and consumers).
pub mod headers {
    /// JetStream deduplication id: the outbox event id.
    pub const MSG_ID: &str = "Nats-Msg-Id";
    pub const EVENT_TYPE: &str = "x-event-type";
    pub const TENANT_ID: &str = "x-tenant-id";
    pub const CORRELATION_ID: &str = "x-correlation-id";
    pub const AGGREGATE_TYPE: &str = "x-aggregate-type";
    pub const AGGREGATE_ID: &str = "x-aggregate-id";
    pub const OCCURRED_AT: &str = "x-occurred-at";
    pub const TRACEPARENT: &str = "traceparent";
}

/// An outbox row on its way to the broker.
#[derive(Debug, Clone)]
pub struct OutboundEvent {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub event_type: String,
    pub aggregate_type: String,
    pub aggregate_id: Uuid,
    pub payload: serde_json::Value,
    pub correlation_id: Uuid,
    pub traceparent: Option<String>,
    pub occurred_at: DateTime<Utc>,
}

/// A delivered event, decoded from broker headers and payload.
#[derive(Debug, Clone)]
pub struct InboundEvent {
    pub id: Uuid,
    pub event_type: String,
    pub tenant_id: TenantId,
    pub correlation_id: CorrelationId,
    pub aggregate_id: Uuid,
    pub traceparent: Option<String>,
    pub payload: serde_json::Value,
}

#[derive(Debug, thiserror::Error)]
#[error("malformed event envelope: {0}")]
pub struct EnvelopeError(String);

impl InboundEvent {
    /// Decodes the envelope. A malformed envelope is a permanent failure.
    pub fn from_parts(headers: &HeaderMap, payload: &[u8]) -> Result<Self, EnvelopeError> {
        let text = |name: &str| {
            headers
                .get(name)
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| EnvelopeError(format!("missing header `{name}`")))
        };
        let uuid = |name: &str| {
            text(name).and_then(|v| {
                Uuid::parse_str(v)
                    .map_err(|_| EnvelopeError(format!("header `{name}` is not a UUID")))
            })
        };
        Ok(Self {
            id: uuid(headers::MSG_ID)?,
            event_type: text(headers::EVENT_TYPE)?.to_owned(),
            tenant_id: TenantId::from_uuid(uuid(headers::TENANT_ID)?),
            correlation_id: CorrelationId::from_uuid(uuid(headers::CORRELATION_ID)?),
            aggregate_id: uuid(headers::AGGREGATE_ID)?,
            traceparent: text(headers::TRACEPARENT).ok().map(str::to_owned),
            payload: serde_json::from_slice(payload)
                .map_err(|err| EnvelopeError(format!("payload is not JSON: {err}")))?,
        })
    }

    /// Deserializes the payload into the handler's event type.
    pub fn decode<T: DeserializeOwned>(&self) -> Result<T, serde_json::Error> {
        T::deserialize(&self.payload)
    }
}

impl OutboundEvent {
    /// Header map carrying the envelope (see [`headers`]).
    pub fn header_map(&self) -> HeaderMap {
        let mut map = HeaderMap::new();
        let mut put = |name: &'static str, value: String| {
            if let Ok(value) = value.parse() {
                map.insert(name, value);
            }
        };
        put(headers::MSG_ID, self.id.to_string());
        put(headers::EVENT_TYPE, self.event_type.clone());
        put(headers::TENANT_ID, self.tenant_id.to_string());
        put(headers::CORRELATION_ID, self.correlation_id.to_string());
        put(headers::AGGREGATE_TYPE, self.aggregate_type.clone());
        put(headers::AGGREGATE_ID, self.aggregate_id.to_string());
        put(headers::OCCURRED_AT, self.occurred_at.to_rfc3339());
        if let Some(traceparent) = &self.traceparent {
            put(headers::TRACEPARENT, traceparent.clone());
        }
        map
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_round_trips_through_headers() {
        let outbound = OutboundEvent {
            id: Uuid::now_v7(),
            tenant_id: Uuid::now_v7(),
            event_type: "billing.invoice.issued.v1".to_owned(),
            aggregate_type: "billing.invoice".to_owned(),
            aggregate_id: Uuid::now_v7(),
            payload: serde_json::json!({ "total_minor": 1 }),
            correlation_id: Uuid::now_v7(),
            traceparent: None,
            occurred_at: Utc::now(),
        };

        let inbound = InboundEvent::from_parts(
            &outbound.header_map(),
            &serde_json::to_vec(&outbound.payload).unwrap(),
        )
        .unwrap();

        assert_eq!(inbound.id, outbound.id);
        assert_eq!(inbound.tenant_id.as_uuid(), outbound.tenant_id);
        assert_eq!(inbound.payload, outbound.payload);
    }

    #[test]
    fn missing_tenant_header_is_rejected() {
        let mut headers = HeaderMap::new();
        headers.insert(headers::MSG_ID, Uuid::now_v7().to_string().parse().unwrap());

        assert!(InboundEvent::from_parts(&headers, b"{}").is_err());
    }
}
