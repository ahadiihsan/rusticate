//! `init` installs a process-global subscriber, so this binary holds exactly
//! one test (nextest runs every test in its own process anyway).
#![cfg(test)]

use telemetry::{LogFormat, TelemetryConfig, current_trace_id, current_traceparent};

#[test]
fn every_span_gets_a_trace_id_that_survives_a_process_boundary() {
    let config = TelemetryConfig {
        log_format: LogFormat::Json,
        ..TelemetryConfig::default()
    };
    let guard = telemetry::init("test", &config).unwrap();

    let request = tracing::info_span!("request");
    let (trace_id, traceparent) = request.in_scope(|| (current_trace_id(), current_traceparent()));
    let trace_id = trace_id.expect("an active span has a trace id");
    let traceparent = traceparent.expect("and a traceparent");

    // Another process (the consumer) continues the trace from the stored header.
    let consumer = tracing::info_span!("consumer");
    telemetry::set_parent_from_traceparent(&consumer, &traceparent);
    let continued = consumer.in_scope(current_trace_id);

    assert!(traceparent.contains(&trace_id));
    assert_eq!(continued, Some(trace_id));
    assert!(
        telemetry::init("test", &config).is_err(),
        "init is once per process"
    );
    drop(guard);
}
