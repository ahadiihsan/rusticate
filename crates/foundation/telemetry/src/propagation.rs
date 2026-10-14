//! W3C trace-context propagation over HTTP-style header maps.
//!
//! The same helpers serve inbound HTTP requests (server-kit), outbound
//! messages (outbox relay) and inbound messages (event consumers): NATS
//! headers are converted to an [`http::HeaderMap`] at the edge.

use http::HeaderMap;
use http::header::{HeaderName, HeaderValue};
use opentelemetry::propagation::{Extractor, Injector};
use tracing_opentelemetry::OpenTelemetrySpanExt as _;

/// Reads propagation fields from a [`HeaderMap`].
#[derive(Debug)]
pub struct HeaderMapExtractor<'a>(pub &'a HeaderMap);

impl Extractor for HeaderMapExtractor<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(|value| value.to_str().ok())
    }

    fn keys(&self) -> Vec<&str> {
        self.0.keys().map(HeaderName::as_str).collect()
    }
}

/// Writes propagation fields into a [`HeaderMap`].
#[derive(Debug)]
pub struct HeaderMapInjector<'a>(pub &'a mut HeaderMap);

impl Injector for HeaderMapInjector<'_> {
    fn set(&mut self, key: &str, value: String) {
        if let (Ok(name), Ok(value)) = (
            HeaderName::from_bytes(key.as_bytes()),
            HeaderValue::from_str(&value),
        ) {
            self.0.insert(name, value);
        }
    }
}

/// Makes `span` a child of the remote trace described by `headers`, if any.
pub fn extract_parent(span: &tracing::Span, headers: &HeaderMap) {
    let parent = opentelemetry::global::get_text_map_propagator(|propagator| {
        propagator.extract(&HeaderMapExtractor(headers))
    });
    // An error only means the span is already closed or not an OTel span.
    let _ = span.set_parent(parent);
}

/// Writes the current span's trace context (`traceparent`) into `headers`.
pub fn inject_current(headers: &mut HeaderMap) {
    let context = tracing::Span::current().context();
    opentelemetry::global::get_text_map_propagator(|propagator| {
        propagator.inject_context(&context, &mut HeaderMapInjector(headers));
    });
}

/// The current span's W3C `traceparent`, for storing alongside work that is
/// continued later by another process (outbox rows).
pub fn current_traceparent() -> Option<String> {
    let mut headers = HeaderMap::new();
    inject_current(&mut headers);
    headers
        .get("traceparent")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

/// Makes `span` a child of the trace described by a stored `traceparent`.
pub fn set_parent_from_traceparent(span: &tracing::Span, traceparent: &str) {
    if let Ok(value) = HeaderValue::from_str(traceparent) {
        let mut headers = HeaderMap::new();
        headers.insert("traceparent", value);
        extract_parent(span, &headers);
    }
}
