//! OpenTelemetry OTLP exporter and tracing subscriber wiring.

use apxm_driver::ServerObservabilityConfig;
use opentelemetry::trace::TracerProvider;
use opentelemetry_otlp::WithExportConfig;
use tracing::warn;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// Lightweight handle representing a configured OTLP exporter.
#[derive(Clone)]
pub(crate) struct OtelExporter {
    endpoint: std::sync::Arc<String>,
}

impl OtelExporter {
    pub(crate) fn endpoint(&self) -> &str {
        self.endpoint.as_str()
    }
}

#[derive(Debug)]
pub(crate) struct OtelInitError(pub String);

impl std::fmt::Display for OtelInitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for OtelInitError {}

pub(crate) fn warn_init_failure(error: &OtelInitError) {
    warn!(error = %error, "OTLP exporter init failed; continuing without export");
}

fn json_logs_enabled() -> bool {
    std::env::var("APXM_LOG_FORMAT").is_ok_and(|value| value.trim().eq_ignore_ascii_case("json"))
}

/// Initialize the global tracing subscriber with optional OTLP export.
pub(crate) fn init_tracing_subscriber(
    observability: &ServerObservabilityConfig,
    log_filter: &str,
) -> Result<Option<OtelExporter>, OtelInitError> {
    let filter =
        EnvFilter::try_new(log_filter).map_err(|error| OtelInitError(error.to_string()))?;

    let endpoint = observability
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
        .map_err(|error| OtelInitError(error.to_string()))?;

    let provider = opentelemetry_sdk::trace::TracerProvider::builder()
        .with_batch_exporter(exporter, opentelemetry_sdk::runtime::Tokio)
        .build();

    let tracer = provider.tracer("apxm-server");
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
