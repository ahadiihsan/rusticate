use std::error::Error as StdError;

use crate::OutboundEvent;

/// Failure to publish (part of) a batch. The relay retries the whole batch;
/// already-published events are deduplicated by the broker (event id) and by
/// consumer inboxes.
#[derive(Debug, thiserror::Error)]
#[error("publishing to the event sink failed")]
pub struct SinkError(#[source] pub Box<dyn StdError + Send + Sync>);

/// Destination of relayed events. The broker adapter is the one place a trait
/// is the right tool: it is a real I/O boundary with a test double.
pub trait EventSink: Send + Sync {
    /// Publishes `events` in order and returns once the broker durably
    /// acknowledged every one of them.
    fn publish(
        &self,
        events: &[OutboundEvent],
    ) -> impl Future<Output = Result<(), SinkError>> + Send;
}

/// Records published events in memory; can be told to fail or to be slow.
#[cfg(feature = "testing")]
#[derive(Debug, Clone, Default)]
pub struct MemorySink {
    published: std::sync::Arc<std::sync::Mutex<Vec<OutboundEvent>>>,
    failing: std::sync::Arc<std::sync::atomic::AtomicBool>,
    delay_ms: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

#[cfg(feature = "testing")]
impl MemorySink {
    pub fn published(&self) -> Vec<OutboundEvent> {
        self.published
            .lock()
            .map(|events| events.clone())
            .unwrap_or_default()
    }

    pub fn set_failing(&self, failing: bool) {
        self.failing
            .store(failing, std::sync::atomic::Ordering::SeqCst);
    }

    /// Simulates a slow broker: every publish waits this long first.
    pub fn set_delay(&self, delay: std::time::Duration) {
        let millis = u64::try_from(delay.as_millis()).unwrap_or(u64::MAX);
        self.delay_ms
            .store(millis, std::sync::atomic::Ordering::SeqCst);
    }
}

#[cfg(feature = "testing")]
impl EventSink for MemorySink {
    fn publish(
        &self,
        events: &[OutboundEvent],
    ) -> impl Future<Output = Result<(), SinkError>> + Send {
        let sink = self.clone();
        let events = events.to_vec();
        async move {
            let delay = sink.delay_ms.load(std::sync::atomic::Ordering::SeqCst);
            if delay > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
            }
            if sink.failing.load(std::sync::atomic::Ordering::SeqCst) {
                return Err(SinkError(Box::new(std::io::Error::other(
                    "sink unavailable",
                ))));
            }
            if let Ok(mut published) = sink.published.lock() {
                published.extend(events);
            }
            Ok(())
        }
    }
}
