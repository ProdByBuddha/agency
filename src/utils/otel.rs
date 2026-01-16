//! Professional Observability (OpenTelemetry)
//! 
//! Provides a centralized telemetry system for tracing and metrics collection.
//! Derived from codex-rs patterns.
//! Now includes Log Rotation (The Excretory System).

use opentelemetry::{global, KeyValue};
use opentelemetry_sdk::{propagation::TraceContextPropagator, runtime, trace as sdktrace, Resource};
use opentelemetry::trace::TracerProvider;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Registry};
use tracing_appender::non_blocking::WorkerGuard;
use std::error::Error;

pub struct OtelGuard {
    _log_guard: WorkerGuard,
}

impl Drop for OtelGuard {
    fn drop(&mut self) {
        global::shutdown_tracer_provider();
    }
}

pub fn init_telemetry(service_name: &str) -> Result<OtelGuard, Box<dyn Error>> {
    global::set_text_map_propagator(TraceContextPropagator::new());

    // 1. Configure OTLP Span Exporter
    let exporter = opentelemetry_otlp::new_exporter()
        .tonic()
        .build_span_exporter()?;

    // 2. Configure Tracer Provider
    let trace_config = sdktrace::Config::default().with_resource(Resource::new(vec![
        KeyValue::new("service.name", service_name.to_string()),
        KeyValue::new("environment", "production"),
    ]));

    let provider = sdktrace::TracerProvider::builder()
        .with_batch_exporter(exporter, runtime::Tokio)
        .with_config(trace_config)
        .build();

    global::set_tracer_provider(provider.clone());
    
    let tracer = provider.tracer(service_name.to_string());
    let telemetry = tracing_opentelemetry::layer().with_tracer(tracer);

    // 3. Configure Log Rotation (The Excretory System)
    // Rotates logs daily, ensuring we don't fill the disk indefinitely.
    let file_appender = tracing_appender::rolling::daily("logs", "agency.log");
    let (non_blocking, log_guard) = tracing_appender::non_blocking(file_appender);

    // 4. Configure Filter
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("rust_agency=info,opentelemetry=error"));

    // 5. Initialize Global Subscriber
    // We compose:
    // - Stdout layer (for immediate feedback)
    // - File layer (for long-term history)
    // - OpenTelemetry layer (for distributed tracing)
    Registry::default()
        .with(filter)
        .with(telemetry)
        .with(tracing_subscriber::fmt::layer().with_target(false))
        .with(tracing_subscriber::fmt::layer().with_writer(non_blocking).with_ansi(false))
        .init();

    Ok(OtelGuard { _log_guard: log_guard })
}