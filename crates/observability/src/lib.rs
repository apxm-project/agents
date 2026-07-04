//! `apxm-observability` — one telemetry bootstrap for every APXM service.
//!
//! Extracted from `apxm-server`'s original `observability.rs` (OBS-3) so
//! server/os/auth/studio share exactly one definition of:
//!
//! - structured logging (`tracing_subscriber`, plain or `APXM_LOG_FORMAT=json`)
//! - OTLP trace export (`tracing-opentelemetry` + `opentelemetry-otlp`), when
//!   an endpoint is configured
//! - W3C `traceparent` extraction from an inbound request and propagation
//!   into outgoing spans/requests, so a trace stays one continuous trace
//!   across every service-to-service hop (joint acceptance with HOST WS-B,
//!   already landed in `os` — see `os-listeners::relay_http`).
//!
//! Each service keeps its own `main`/service-startup call site; this crate
//! only owns the wiring, not the config type (services already have their
//! own process config — see [`Config`] for the minimal shape this crate
//! needs from it).

mod metrics;
mod traceparent;

pub use metrics::AppMetrics;
pub use traceparent::{
    extract_traceparent, inject_current_traceparent, traceparent_from_headers,
};

#[cfg(feature = "reqwest")]
pub use traceparent::inject_traceparent_reqwest;

use opentelemetry::trace::TracerProvider;
use opentelemetry_otlp::WithExportConfig;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// Minimal, service-agnostic telemetry configuration. Services map their own
/// process config into this at startup (e.g. server's
/// `ServerObservabilityConfig` -> [`Config`]).
#[derive(Debug, Clone)]
pub struct Config {
    /// Logical service name, used as the OTel tracer name (e.g.
    /// `"apxm-server"`, `"apxm-os"`, `"apxm-auth"`, `"apxm-studio"`).
    pub service_name: &'static str,
    /// `tracing_subscriber::EnvFilter` directive string (e.g. `"info"`).
    pub log_filter: String,
    /// OTLP/HTTP trace exporter endpoint. `None` disables trace export —
    /// logging still initializes.
    pub otlp_endpoint: Option<String>,
}

/// Lightweight handle representing a configured OTLP exporter.
#[derive(Clone)]
pub struct OtelExporter {
    endpoint: std::sync::Arc<String>,
}

impl OtelExporter {
    pub fn endpoint(&self) -> &str {
        self.endpoint.as_str()
    }
}

#[derive(Debug)]
pub struct InitError(pub String);

impl std::fmt::Display for InitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for InitError {}

/// Log a `warn!` for an OTLP init failure without aborting the process —
/// every service degrades to plain/JSON logging without export.
pub fn warn_init_failure(error: &InitError) {
    tracing::warn!(error = %error, "OTLP exporter init failed; continuing without export");
}

/// `true` when `APXM_LOG_FORMAT=json` (case-insensitive) — the single env
/// var every service checks for uniform structured logs.
pub fn json_logs_enabled() -> bool {
    std::env::var("APXM_LOG_FORMAT").is_ok_and(|value| value.trim().eq_ignore_ascii_case("json"))
}

/// Initialize the global tracing subscriber with optional OTLP export, and
/// register the W3C `traceparent` (`tracecontext`) propagator globally so
/// [`extract_traceparent`]/[`inject_current_traceparent`] round-trip through
/// the same format every other OTel-instrumented hop uses.
///
/// Call once, at process startup, before any other tracing/otel call.
pub fn init(config: &Config) -> Result<Option<OtelExporter>, InitError> {
    opentelemetry::global::set_text_map_propagator(
        opentelemetry_sdk::propagation::TraceContextPropagator::new(),
    );

    let filter =
        EnvFilter::try_new(&config.log_filter).map_err(|error| InitError(error.to_string()))?;

    let endpoint = config
        .otlp_endpoint
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());

    let Some(endpoint) = endpoint else {
        if json_logs_enabled() {
            tracing_subscriber::registry()
                .with(filter)
                .with(tracing_subscriber::fmt::layer().json())
                .init();
        } else {
            tracing_subscriber::registry()
                .with(filter)
                .with(tracing_subscriber::fmt::layer())
                .init();
        }
        return Ok(None);
    };

    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_http()
        .with_endpoint(endpoint)
        .build()
        .map_err(|error| InitError(error.to_string()))?;

    let provider = opentelemetry_sdk::trace::TracerProvider::builder()
        .with_batch_exporter(exporter, opentelemetry_sdk::runtime::Tokio)
        .build();

    let tracer = provider.tracer(config.service_name);
    let telemetry = tracing_opentelemetry::layer().with_tracer(tracer);

    if json_logs_enabled() {
        tracing_subscriber::registry()
            .with(filter)
            .with(telemetry)
            .with(tracing_subscriber::fmt::layer().json())
            .init();
    } else {
        tracing_subscriber::registry()
            .with(filter)
            .with(telemetry)
            .with(tracing_subscriber::fmt::layer())
            .init();
    }

    Ok(Some(OtelExporter {
        endpoint: std::sync::Arc::new(endpoint.to_string()),
    }))
}

/// Initialize OTLP metric export (OBS-4 decision-5) and return the
/// decision-5 [`AppMetrics`] instrument set.
///
/// Reuses `config.otlp_endpoint` — the same OTLP/HTTP collector that
/// receives traces also receives metrics, which is normal OTLP practice
/// (one collector, multiple signal pipelines). When no endpoint is
/// configured, metrics are still instrumented (so call sites never need to
/// branch on whether export is enabled) but are recorded into a
/// non-exporting meter provider and dropped — mirrors [`init`]'s "degrade to
/// no export, never panic" contract.
///
/// Call once, at process startup — independent of [`init`] (traces); a
/// service typically calls both.
pub fn init_metrics(config: &Config) -> Result<AppMetrics, InitError> {
    let endpoint = config
        .otlp_endpoint
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());

    let provider = match endpoint {
        Some(endpoint) => {
            let exporter = opentelemetry_otlp::MetricExporter::builder()
                .with_http()
                .with_endpoint(endpoint)
                .build()
                .map_err(|error| InitError(error.to_string()))?;

            let reader = opentelemetry_sdk::metrics::PeriodicReader::builder(
                exporter,
                opentelemetry_sdk::runtime::Tokio,
            )
            .build();

            opentelemetry_sdk::metrics::SdkMeterProvider::builder()
                .with_reader(reader)
                .build()
        }
        // No endpoint configured: build a meter provider with no readers.
        // Instruments still work (recording is a no-op cost, not an error);
        // nothing is exported anywhere.
        None => opentelemetry_sdk::metrics::SdkMeterProvider::builder().build(),
    };

    use opentelemetry::metrics::MeterProvider as _;

    opentelemetry::global::set_meter_provider(provider.clone());

    let meter = provider.meter(config.service_name);
    Ok(AppMetrics::new(&meter))
}
