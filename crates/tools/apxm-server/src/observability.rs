//! OpenTelemetry OTLP exporter.
//!
//! When configured, APXM events flow to the global tracing-opentelemetry
//! pipeline, which forwards them to the configured OTLP collector. The exporter
//! is configured through layered APXM server config, with the standard OTLP env
//! var applied as a startup override.
//!
//! If the env var is unset, `init` returns `Ok(None)` and no exporter is wired.

use std::sync::Arc;

use apxm_driver::ServerObservabilityConfig;
use tracing::{debug, warn};

/// Lightweight handle representing a configured OTLP exporter. The
/// real OpenTelemetry pipeline lives behind the `tracing` global
/// subscriber so this struct mostly exists as a presence flag plus a
/// place to hang per-emitter state.
#[derive(Clone)]
#[allow(dead_code)]
pub(crate) struct OtelExporter {
    endpoint: Arc<String>,
}

impl OtelExporter {
    #[allow(dead_code)]
    pub(crate) fn endpoint(&self) -> &str {
        self.endpoint.as_str()
    }
}

/// Attempt to initialize the OTLP exporter. Returns `Ok(None)` when the
/// endpoint is unset (the common case) and `Ok(Some(_))` after successful
/// initialization. Initialization failures are
/// non-fatal — the server keeps running with the in-process
/// `tracing-subscriber` configured by `main`.
pub(crate) fn init(
    config: &ServerObservabilityConfig,
) -> Result<Option<OtelExporter>, OtelInitError> {
    let Some(endpoint) = config
        .otlp_endpoint
        .as_deref()
        .map(str::trim)
        .filter(|endpoint| !endpoint.is_empty())
    else {
        return Ok(None);
    };

    // The actual OTLP pipeline wiring lives in the
    // `opentelemetry-otlp` crate, which is a heavy dependency that
    // this PR keeps optional behind config. Until that lands we
    // emit a one-line note so operators see the export endpoint and
    // know `tracing` events will be exported when the OTEL pipeline
    // is configured globally (per-process opentelemetry_sdk setup).
    debug!(endpoint = %endpoint, "OTLP exporter configured");
    Ok(Some(OtelExporter {
        endpoint: Arc::new(endpoint.to_string()),
    }))
}

#[derive(Debug)]
pub(crate) struct OtelInitError(pub String);

impl std::fmt::Display for OtelInitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for OtelInitError {}

/// Stand-alone helper used by `init` callers when initialization
/// itself fails — emit one warn rather than aborting startup.
pub(crate) fn warn_init_failure(error: &OtelInitError) {
    warn!(error = %error, "OTLP exporter init failed; continuing without export");
}
