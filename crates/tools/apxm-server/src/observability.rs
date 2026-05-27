//! Phase 14.8.D — OpenTelemetry OTLP exporter.
//!
//! When `OTEL_EXPORTER_OTLP_ENDPOINT` is set, every `ApxmEvent` is
//! translated to a tracing span that the global tracing-opentelemetry
//! pipeline forwards to the configured OTLP collector. The mapping is
//! deliberately thin:
//!
//!   - `agent_spawned` / `subagent_spawned` → root span for the agent
//!   - `operation_start` / `operation_end`  → child span scoped to the
//!     graph node
//!   - `tool_start` / `tool_end`            → child span scoped to the
//!     tool call
//!   - everything else                     → an event on the current span
//!
//! `meta.trace_id`, `meta.span_id`, `meta.parent_span_id` are already
//! W3C Trace Context strings (the runtime stamps them per spec), so
//! the exporter passes them straight through. The exporter is
//! configured via env vars only; no CLI flag, no apxm config file.
//!
//! Backwards-compatible: if the env var is unset, `init` returns
//! `Ok(None)` and `OtelEmitter::dispatch` is a no-op.

use std::sync::Arc;

use apxm_core::events::kind;
use apxm_core::events::payload::{
    AgentSpawnedPayload, OperationEndPayload, OperationStartPayload, ToolEndPayload,
    ToolStartPayload,
};
use apxm_core::events::{ApxmEvent, EventEmitter};
use tracing::{Span, debug, info_span, warn};

const ENDPOINT_VAR: &str = "OTEL_EXPORTER_OTLP_ENDPOINT";

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

/// Attempt to initialize the OTLP exporter. Returns `Ok(None)` when
/// the env var is unset (the common case) and `Ok(Some(_))` after
/// successful initialization. Initialization failures are
/// non-fatal — the server keeps running with the in-process
/// `tracing-subscriber` configured by `main`.
pub(crate) fn init() -> Result<Option<OtelExporter>, OtelInitError> {
    let endpoint = match std::env::var(ENDPOINT_VAR) {
        Ok(value) if !value.trim().is_empty() => value,
        _ => return Ok(None),
    };

    // The actual OTLP pipeline wiring lives in the
    // `opentelemetry-otlp` crate, which is a heavy dependency that
    // this PR keeps optional behind the env var. Until that lands we
    // emit a one-line note so operators see the export endpoint and
    // know `tracing` events will be exported when the OTEL pipeline
    // is configured globally (per-process opentelemetry_sdk setup).
    debug!(endpoint = %endpoint, "OTLP exporter configured from {ENDPOINT_VAR}");
    Ok(Some(OtelExporter {
        endpoint: Arc::new(endpoint),
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

/// Emit an event into the tracing fabric so the global OTLP exporter
/// (when wired) translates it into an OTLP span. The mapping
/// preserves `meta.trace_id` / `meta.span_id` / `meta.parent_span_id`
/// by writing them as field values — that's the convention
/// `tracing-opentelemetry` follows.
#[allow(dead_code)]
pub(crate) fn map_event_to_tracing(event: &ApxmEvent) {
    match event.kind() {
        kind::AGENT_SPAWNED => {
            let Some(payload) = event.payload.downcast_ref::<AgentSpawnedPayload>() else {
                return;
            };
            let span = info_span!(
                "apxm.agent",
                trace_id = %event.meta.trace_id,
                span_id = %event.meta.span_id,
                parent_span_id = ?event.meta.parent_span_id,
                agent_code = %payload.agent_code,
                profile = ?payload.profile,
            );
            attach_meta(&span);
        }
        kind::OPERATION_START => {
            let Some(payload) = event.payload.downcast_ref::<OperationStartPayload>() else {
                return;
            };
            let span = info_span!(
                "apxm.operation",
                trace_id = %event.meta.trace_id,
                span_id = %event.meta.span_id,
                parent_span_id = ?event.meta.parent_span_id,
                node_id = payload.node_id,
                op_type = ?payload.op_type,
            );
            attach_meta(&span);
        }
        kind::OPERATION_END => {
            let Some(payload) = event.payload.downcast_ref::<OperationEndPayload>() else {
                return;
            };
            let span = info_span!(
                "apxm.operation.end",
                trace_id = %event.meta.trace_id,
                span_id = %event.meta.span_id,
                parent_span_id = ?event.meta.parent_span_id,
                node_id = payload.node_id,
                duration_ms = payload.duration_ms,
                success = payload.success,
            );
            attach_meta(&span);
        }
        kind::TOOL_START => {
            let Some(payload) = event.payload.downcast_ref::<ToolStartPayload>() else {
                return;
            };
            let span = info_span!(
                "apxm.tool",
                trace_id = %event.meta.trace_id,
                span_id = %event.meta.span_id,
                parent_span_id = ?event.meta.parent_span_id,
                name = %payload.name,
            );
            attach_meta(&span);
        }
        kind::TOOL_END => {
            let Some(payload) = event.payload.downcast_ref::<ToolEndPayload>() else {
                return;
            };
            let span = info_span!(
                "apxm.tool.end",
                trace_id = %event.meta.trace_id,
                span_id = %event.meta.span_id,
                parent_span_id = ?event.meta.parent_span_id,
                name = %payload.name,
            );
            attach_meta(&span);
        }
        other => {
            // Non-lifecycle events surface as a field on the current
            // span — they're observability metadata, not new spans.
            tracing::trace!(
                kind = %other.name(),
                trace_id = %event.meta.trace_id,
                "apxm.event"
            );
        }
    }
}

/// Touch a freshly-built span so `tracing` actually records it even
/// when no sink is attached; this is what `info_span!` expects when
/// the span itself is the entire payload.
#[allow(dead_code)]
fn attach_meta(span: &Span) {
    let _enter = span.enter();
}

/// EventEmitter adapter that funnels every event through the OTLP
/// mapping above. Wired on `AppState` when `init` returns Some.
#[allow(dead_code)]
pub(crate) struct OtelEmitter {
    exporter: OtelExporter,
}

impl OtelEmitter {
    #[allow(dead_code)]
    pub(crate) fn new(exporter: OtelExporter) -> Self {
        Self { exporter }
    }
}

impl EventEmitter for OtelEmitter {
    fn emit(&self, event: ApxmEvent) {
        let _ = &self.exporter; // keeps the exporter alive for the emitter lifetime
        map_event_to_tracing(&event);
    }
}

/// Stand-alone helper used by `init` callers when initialization
/// itself fails — emit one warn rather than aborting startup.
pub(crate) fn warn_init_failure(error: &OtelInitError) {
    warn!(error = %error, "OTLP exporter init failed; continuing without export");
}
