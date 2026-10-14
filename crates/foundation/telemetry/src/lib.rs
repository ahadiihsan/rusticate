//! Process-wide telemetry (SOP OBS-001..OBS-005).
//!
//! * Structured logs (JSON in production) through a non-blocking writer, so a
//!   slow stdout can never stall request handling.
//! * An OpenTelemetry layer is **always** installed, even without an exporter,
//!   so every span has a real W3C trace id. That id is what clients see in
//!   `problem+json` error bodies ([`current_trace_id`]), and what support uses to
//!   find the request in logs and traces.
//! * W3C `traceparent` propagation helpers for HTTP headers and message headers.

mod propagation;
mod sensitive;

pub use propagation::{
    HeaderMapExtractor, HeaderMapInjector, current_traceparent, extract_parent, inject_current,
    set_parent_from_traceparent,
};
pub use sensitive::Sensitive;

use opentelemetry::trace::{TraceContextExt as _, TracerProvider as _};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::propagation::TraceContextPropagator;
use opentelemetry_sdk::trace::{Sampler, SdkTracerProvider};
use serde::Deserialize;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_opentelemetry::OpenTelemetrySpanExt as _;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;
use tracing_subscriber::{EnvFilter, Layer as _};

/// Log output encoding.
#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LogFormat {
    /// One JSON object per line. The production default.
    #[default]
    Json,
    /// Human-readable, colored output for local development.
    Pretty,
}

/// Telemetry configuration, embedded in each binary's config struct.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TelemetryConfig {
    #[serde(default)]
    pub log_format: LogFormat,
    /// `EnvFilter` directives. `RUST_LOG`, when set, takes precedence.
    #[serde(default = "default_log_filter")]
    pub log_filter: String,
    /// OTLP/gRPC collector endpoint, e.g. `http://otel-collector:4317`.
    /// When absent, spans are created (for trace ids) but not exported.
    #[serde(default)]
    pub otlp_endpoint: Option<String>,
    /// Head sampling ratio for root spans in `[0.0, 1.0]`. Parent decisions win.
    #[serde(default = "default_sample_ratio")]
    pub trace_sample_ratio: f64,
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            log_format: LogFormat::default(),
            log_filter: default_log_filter(),
            otlp_endpoint: None,
            trace_sample_ratio: default_sample_ratio(),
        }
    }
}

fn default_log_filter() -> String {
    "info".to_owned()
}

const fn default_sample_ratio() -> f64 {
    1.0
}

#[derive(Debug, thiserror::Error)]
pub enum TelemetryError {
    #[error("trace_sample_ratio must be within [0.0, 1.0], got {0}")]
    InvalidSampleRatio(f64),
    #[error("invalid log filter directives: {0}")]
    InvalidFilter(#[from] tracing_subscriber::filter::ParseError),
    #[error("failed to build the OTLP exporter: {0}")]
    Exporter(#[from] opentelemetry_otlp::ExporterBuildError),
    #[error("a global tracing subscriber is already installed: {0}")]
    AlreadyInitialized(#[from] tracing_subscriber::util::TryInitError),
}

/// Flushes buffered logs and spans when dropped. Keep it alive for the whole
/// lifetime of `main`.
#[must_use = "dropping the guard immediately stops log and trace export"]
pub struct TelemetryGuard {
    provider: SdkTracerProvider,
    _log_writer: WorkerGuard,
}

impl std::fmt::Debug for TelemetryGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TelemetryGuard").finish_non_exhaustive()
    }
}

impl Drop for TelemetryGuard {
    fn drop(&mut self) {
        if let Err(err) = self.provider.shutdown() {
            tracing::warn!(error = %err, "tracer provider shutdown failed");
        }
    }
}

/// Installs the global subscriber. Call exactly once, first thing in `main`.
pub fn init(
    service_name: &'static str,
    cfg: &TelemetryConfig,
) -> Result<TelemetryGuard, TelemetryError> {
    if !(0.0..=1.0).contains(&cfg.trace_sample_ratio) {
        return Err(TelemetryError::InvalidSampleRatio(cfg.trace_sample_ratio));
    }

    let filter = match EnvFilter::try_from_default_env() {
        Ok(from_env) => from_env,
        Err(_) => EnvFilter::try_new(&cfg.log_filter)?,
    };

    let provider = build_tracer_provider(service_name, cfg)?;
    let tracer = provider.tracer(service_name);
    opentelemetry::global::set_text_map_propagator(TraceContextPropagator::new());

    let (writer, log_guard) = tracing_appender::non_blocking(std::io::stdout());
    let fmt_layer = match cfg.log_format {
        LogFormat::Json => tracing_subscriber::fmt::layer()
            .json()
            .flatten_event(true)
            .with_current_span(true)
            .with_span_list(false)
            .with_writer(writer)
            .boxed(),
        LogFormat::Pretty => tracing_subscriber::fmt::layer().with_writer(writer).boxed(),
    };

    tracing_subscriber::registry()
        .with(filter)
        .with(fmt_layer)
        .with(tracing_opentelemetry::layer().with_tracer(tracer))
        .try_init()?;
    install_panic_hook();

    Ok(TelemetryGuard {
        provider,
        _log_writer: log_guard,
    })
}

/// Routes panics through `tracing` so they land in structured logs with the
/// active span (request id, tenant) instead of raw stderr.
fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let location = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_default();
        tracing::error!(panic = %info, location, "panic");
    }));
}

fn build_tracer_provider(
    service_name: &'static str,
    cfg: &TelemetryConfig,
) -> Result<SdkTracerProvider, TelemetryError> {
    let resource = Resource::builder().with_service_name(service_name).build();
    let sampler =
        Sampler::ParentBased(Box::new(Sampler::TraceIdRatioBased(cfg.trace_sample_ratio)));
    let builder = SdkTracerProvider::builder()
        .with_resource(resource)
        .with_sampler(sampler);

    let builder = match &cfg.otlp_endpoint {
        Some(endpoint) => {
            use opentelemetry_otlp::WithExportConfig as _;
            let exporter = opentelemetry_otlp::SpanExporter::builder()
                .with_tonic()
                .with_endpoint(endpoint.clone())
                .build()?;
            builder.with_batch_exporter(exporter)
        }
        None => builder,
    };
    Ok(builder.build())
}

/// W3C trace id (32 lowercase hex chars) of the current span, if one is active.
///
/// Used for the `trace_id` member of `problem+json` responses so the value a
/// client reports matches the trace in the observability backend.
pub fn current_trace_id() -> Option<String> {
    let context = tracing::Span::current().context();
    let span_context = context.span().span_context().clone();
    span_context
        .is_valid()
        .then(|| span_context.trace_id().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_out_of_range_sample_ratio() {
        let cfg = TelemetryConfig {
            trace_sample_ratio: 1.5,
            ..TelemetryConfig::default()
        };
        assert!(matches!(
            init("test", &cfg),
            Err(TelemetryError::InvalidSampleRatio(_))
        ));
    }

    #[test]
    fn no_trace_id_without_active_span() {
        assert_eq!(current_trace_id(), None);
    }
}
