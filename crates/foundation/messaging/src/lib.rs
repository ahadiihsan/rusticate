//! Asynchronous messaging (SOP MSG-001..MSG-008).
//!
//! ```text
//!  workflow ──(same tx)──▶ platform.outbox_events ──relay──▶ JetStream ──▶ consumer
//!                                                                          │
//!                         platform.inbox_events ◀──(same tx as effects)────┘
//! ```
//!
//! * [`OutboxRelay`]: drains the outbox in id order and publishes with the
//!   event id as the broker deduplication id. One active relay at a time
//!   (transaction-scoped advisory lock); extra replicas are hot standbys.
//! * [`EventHandler`] + [`process`]: at-least-once delivery turned into
//!   exactly-once *effects* via the inbox table.
//! * [`nats`]: the JetStream adapter (stream setup, sink, consumer loop). Kept
//!   thin: every decision above is testable without a broker.
//! * [`maintenance`]: retention for idempotency keys and inbox rows.

mod consumer;
mod envelope;
pub mod maintenance;
pub mod nats;
mod relay;
mod sink;

pub use consumer::{EventHandler, HandlerError, Outcome, process};
pub use envelope::{InboundEvent, OutboundEvent, headers};
pub use relay::{OutboxRelay, RelayConfig, RelayError};
#[cfg(feature = "testing")]
pub use sink::MemorySink;
pub use sink::{EventSink, SinkError};
