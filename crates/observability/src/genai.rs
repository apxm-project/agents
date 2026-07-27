//! Optional OTel GenAI spans emitted from the APXM event bus.
//!
//! The exporter is deliberately an ordinary [`EventEmitter`]. It has no
//! execution authority and no error path back to the runtime: registration is
//! optional, event attributes are bounded to known fields, and raw content is
//! redacted before it can become span data unless an operator opts in.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::SystemTime;

use apxm_core::events::kind::EventKind;
use apxm_core::events::payload::{
    AgentMessagePayload, ExecuteCompletePayload, GenerationIdentity, LlmDonePayload,
    LlmPromptPayload, LlmStepCompletedPayload, NodeOutputPayload, RedactedContent,
    SubagentLlmCallBeginPayload, SubagentLlmCallEndPayload, SubagentSpawnBeginPayload,
    ThoughtPayload, TokenPayload, TokenUsagePayload, ToolCallBeginPayload, ToolCallEndPayload,
    ToolCallPayload, ToolEndPayload, ToolStartPayload,
};
use apxm_core::events::{ApxmEvent, EventEmitter, EventSource};
use blake3::Hasher;
use opentelemetry::trace::{
    Span as _, SpanContext, SpanId, SpanKind, TraceContextExt, TraceFlags, TraceId, TraceState,
    Tracer as _, TracerProvider as _,
};
use opentelemetry::{Context, InstrumentationScope, KeyValue};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::trace::{Tracer, TracerProvider};

/// Pinned GenAI semantic-convention schema URL.
///
/// Verified on 2026-07-10 against the primary OpenTelemetry
/// `semantic-conventions-genai` manifest. That manifest identifies this exact
/// schema URL and records the filtered upstream semantic-conventions input as
/// v1.41.0. A schema bump is therefore an explicit exporter change.
pub const GENAI_SEMCONV_SCHEMA_URL: &str = "https://opentelemetry.io/schemas/gen-ai-dev/1.42.0-dev";

const GENAI_INSTRUMENTATION_SCOPE: &str = "apxm.genai.event-exporter";
const GENAI_REDACTED_CONTENT_TYPE: &str = "redacted";
const MAX_PENDING_CORRELATIONS: usize = 1_024;

/// OTel GenAI operation names used by the event-to-span mapping.
pub const GENAI_OPERATION_CHAT: &str = "chat";
pub const GENAI_OPERATION_CREATE_AGENT: &str = "create_agent";
pub const GENAI_OPERATION_EXECUTE_TOOL: &str = "execute_tool";
pub const GENAI_OPERATION_INVOKE_AGENT: &str = "invoke_agent";
pub const GENAI_OPERATION_INVOKE_WORKFLOW: &str = "invoke_workflow";
pub const GENAI_OPERATION_EVENT: &str = "apxm.event";

/// Registration and privacy controls for the GenAI event exporter.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GenAiExporterConfig {
    /// Whether the exporter is active. The default is disabled so constructing
    /// the adapter cannot change execution or create telemetry by accident.
    pub enabled: bool,
    /// Whether raw message/tool content may be included in span attributes.
    /// The default is false; hashes and shape metadata remain available.
    pub capture_message_content: bool,
}

impl GenAiExporterConfig {
    /// Return the safe, explicitly enabled configuration.
    pub const fn enabled() -> Self {
        Self {
            enabled: true,
            capture_message_content: false,
        }
    }

    /// Enable or disable raw content capture without changing registration.
    pub const fn with_content_capture(self, capture_message_content: bool) -> Self {
        Self {
            capture_message_content,
            ..self
        }
    }
}

/// Build the resource used by the shared OTel provider.
///
/// `apxm-observability::init` uses this resource for the request-span
/// provider, and the Server-side wiring can obtain that same provider through
/// [`crate::OtelExporter::tracer_provider`] before constructing this adapter.
pub fn genai_resource(service_name: impl Into<String>) -> Resource {
    Resource::from_schema_url(
        vec![KeyValue::new("service.name", service_name.into())],
        GENAI_SEMCONV_SCHEMA_URL,
    )
}

/// An optional [`EventEmitter`] that correlates APXM lifecycle events into OTel spans.
///
/// The adapter owns only a named tracer from the caller-provided provider. It
/// does not install a global provider, alter the event, or return export errors
/// to the execution path. Callers can therefore fan it out beside their normal
/// event sink and remove it without changing runtime semantics.
#[derive(Clone, Debug)]
pub struct GenAiEventExporter {
    inner: Arc<GenAiEventExporterInner>,
}

#[derive(Debug)]
struct GenAiEventExporterInner {
    tracer: Tracer,
    config: GenAiExporterConfig,
    correlations: Mutex<CorrelationState>,
}

impl GenAiEventExporter {
    /// Create an exporter from an existing provider.
    ///
    /// The provider should carry [`GENAI_SEMCONV_SCHEMA_URL`] on its resource;
    /// [`genai_resource`] is the canonical helper for that setup. The named
    /// instrumentation scope is also pinned so downstream consumers retain the
    /// schema identity even when they surface scope metadata separately.
    pub fn new(provider: &TracerProvider, config: GenAiExporterConfig) -> Self {
        Self::new_with_max_pending(provider, config, MAX_PENDING_CORRELATIONS)
    }

    fn new_with_max_pending(
        provider: &TracerProvider,
        config: GenAiExporterConfig,
        max_pending: usize,
    ) -> Self {
        let scope = InstrumentationScope::builder(GENAI_INSTRUMENTATION_SCOPE)
            .with_version(env!("CARGO_PKG_VERSION"))
            .with_schema_url(GENAI_SEMCONV_SCHEMA_URL)
            .build();

        Self {
            inner: Arc::new(GenAiEventExporterInner {
                tracer: provider.tracer_with_scope(scope),
                config,
                correlations: Mutex::new(CorrelationState::new(max_pending)),
            }),
        }
    }

    /// Return the immutable adapter configuration.
    pub fn config(&self) -> GenAiExporterConfig {
        self.inner.config
    }

    /// Whether this adapter will emit spans.
    pub fn is_enabled(&self) -> bool {
        self.inner.config.enabled
    }

    /// Export all currently unmatched lifecycle events as instantaneous spans.
    ///
    /// Call this before shutting down the tracer provider so incomplete work is
    /// retained without assigning it a synthetic duration. Dropping the last
    /// exporter handle performs the same drain as a final safeguard.
    pub fn flush_pending(&self) {
        if !self.inner.config.enabled {
            return;
        }

        let pending = lock_correlations(&self.inner.correlations).drain();
        for pending in pending {
            export_emission(
                &self.inner.tracer,
                self.inner.config,
                SpanEmission::Unmatched {
                    pending,
                    outcome: CorrelationOutcome::ExporterShutdown,
                },
            );
        }
    }

    #[cfg(test)]
    fn pending_correlation_count(&self) -> usize {
        lock_correlations(&self.inner.correlations).pending.len()
    }
}

impl EventEmitter for GenAiEventExporter {
    fn emit(&self, event: ApxmEvent) {
        if !self.inner.config.enabled {
            return;
        }

        let emissions = lock_correlations(&self.inner.correlations).accept(event);
        for emission in emissions {
            export_emission(&self.inner.tracer, self.inner.config, emission);
        }
    }
}

impl Drop for GenAiEventExporter {
    fn drop(&mut self) {
        if Arc::strong_count(&self.inner) != 1 || !self.inner.config.enabled {
            return;
        }

        self.flush_pending();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CorrelationRole {
    Begin,
    End,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CorrelationFamily {
    Operation,
    ModelStep,
    Inference,
    AgentTool,
    Tool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CorrelationIdentity {
    Operation { node_id: u64, op_type: String },
    Generation(GenerationIdentity),
    ToolCall(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CorrelationKey {
    family: CorrelationFamily,
    trace_id: String,
    parent_span_id: Option<String>,
    identity: CorrelationIdentity,
}

#[derive(Debug, Clone)]
struct CorrelationDescriptor {
    key: CorrelationKey,
    role: CorrelationRole,
    span_name: &'static str,
    operation: &'static str,
    span_kind: SpanKind,
}

#[derive(Debug)]
struct PendingCorrelation {
    event: ApxmEvent,
    descriptor: CorrelationDescriptor,
}

#[derive(Debug, Clone, Copy)]
enum CorrelationOutcome {
    CapacityEvicted,
    ExporterShutdown,
    InvalidTimestampOrder,
}

impl CorrelationOutcome {
    const fn as_str(self) -> &'static str {
        match self {
            Self::CapacityEvicted => "capacity_evicted",
            Self::ExporterShutdown => "exporter_shutdown",
            Self::InvalidTimestampOrder => "invalid_timestamp_order",
        }
    }
}

#[derive(Debug)]
enum SpanEmission {
    Paired {
        begin: PendingCorrelation,
        end: Box<PendingCorrelation>,
    },
    Unmatched {
        pending: PendingCorrelation,
        outcome: CorrelationOutcome,
    },
    Instant(ApxmEvent),
}

#[derive(Debug)]
struct CorrelationState {
    pending: VecDeque<PendingCorrelation>,
    max_pending: usize,
}

impl CorrelationState {
    fn new(max_pending: usize) -> Self {
        Self {
            pending: VecDeque::new(),
            max_pending,
        }
    }

    fn accept(&mut self, event: ApxmEvent) -> Vec<SpanEmission> {
        let Some(descriptor) = correlation_descriptor(&event) else {
            return vec![SpanEmission::Instant(event)];
        };

        let counterpart = self.pending.iter().position(|pending| {
            pending.descriptor.key == descriptor.key && pending.descriptor.role != descriptor.role
        });

        if let Some(index) = counterpart {
            let previous = self
                .pending
                .remove(index)
                .expect("correlation index came from the pending queue");
            let current = PendingCorrelation { event, descriptor };
            let (begin, end) = match current.descriptor.role {
                CorrelationRole::Begin => (current, previous),
                CorrelationRole::End => (previous, current),
            };

            if end.event.meta.timestamp >= begin.event.meta.timestamp {
                return vec![SpanEmission::Paired {
                    begin,
                    end: Box::new(end),
                }];
            }

            return vec![
                SpanEmission::Unmatched {
                    pending: begin,
                    outcome: CorrelationOutcome::InvalidTimestampOrder,
                },
                SpanEmission::Unmatched {
                    pending: end,
                    outcome: CorrelationOutcome::InvalidTimestampOrder,
                },
            ];
        }

        self.pending
            .push_back(PendingCorrelation { event, descriptor });
        let mut emissions = Vec::new();
        while self.pending.len() > self.max_pending {
            if let Some(pending) = self.pending.pop_front() {
                emissions.push(SpanEmission::Unmatched {
                    pending,
                    outcome: CorrelationOutcome::CapacityEvicted,
                });
            }
        }
        emissions
    }

    fn drain(&mut self) -> Vec<PendingCorrelation> {
        self.pending.drain(..).collect()
    }
}

fn lock_correlations(state: &Mutex<CorrelationState>) -> MutexGuard<'_, CorrelationState> {
    state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn correlation_descriptor(event: &ApxmEvent) -> Option<CorrelationDescriptor> {
    let (family, role, identity, span_name, operation, span_kind) =
        if matches!(event.kind().name(), "operation_start" | "operation_end") {
            let (node_id, op_type) = operation_identity(event)?;
            (
                CorrelationFamily::Operation,
                if event.kind().name() == "operation_start" {
                    CorrelationRole::Begin
                } else {
                    CorrelationRole::End
                },
                CorrelationIdentity::Operation { node_id, op_type },
                "apxm.operation",
                GENAI_OPERATION_INVOKE_WORKFLOW,
                SpanKind::Internal,
            )
        } else if let Some(payload) = event.payload.downcast_ref::<SubagentLlmCallBeginPayload>() {
            (
                CorrelationFamily::ModelStep,
                CorrelationRole::Begin,
                CorrelationIdentity::Generation(payload.generation.clone()?),
                "apxm.model.step",
                GENAI_OPERATION_CHAT,
                SpanKind::Internal,
            )
        } else if let Some(payload) = event.payload.downcast_ref::<SubagentLlmCallEndPayload>() {
            (
                CorrelationFamily::ModelStep,
                CorrelationRole::End,
                CorrelationIdentity::Generation(payload.generation.clone()?),
                "apxm.model.step",
                GENAI_OPERATION_CHAT,
                SpanKind::Internal,
            )
        } else if let Some(payload) = event.payload.downcast_ref::<LlmPromptPayload>() {
            (
                CorrelationFamily::Inference,
                CorrelationRole::Begin,
                CorrelationIdentity::Generation(payload.generation.clone()?),
                "apxm.model.inference",
                GENAI_OPERATION_CHAT,
                SpanKind::Client,
            )
        } else if let Some(payload) = event.payload.downcast_ref::<LlmStepCompletedPayload>() {
            (
                CorrelationFamily::Inference,
                CorrelationRole::End,
                CorrelationIdentity::Generation(payload.generation.clone()?),
                "apxm.model.inference",
                GENAI_OPERATION_CHAT,
                SpanKind::Client,
            )
        } else if let Some(payload) = event.payload.downcast_ref::<ToolCallBeginPayload>() {
            (
                CorrelationFamily::AgentTool,
                CorrelationRole::Begin,
                CorrelationIdentity::ToolCall(
                    payload.tool_call_correlation.as_ref()?.tool_call_id.clone(),
                ),
                "apxm.tool.operation",
                GENAI_OPERATION_EXECUTE_TOOL,
                SpanKind::Internal,
            )
        } else if let Some(payload) = event.payload.downcast_ref::<ToolCallEndPayload>() {
            (
                CorrelationFamily::AgentTool,
                CorrelationRole::End,
                CorrelationIdentity::ToolCall(
                    payload.tool_call_correlation.as_ref()?.tool_call_id.clone(),
                ),
                "apxm.tool.operation",
                GENAI_OPERATION_EXECUTE_TOOL,
                SpanKind::Internal,
            )
        } else if let Some(payload) = event.payload.downcast_ref::<ToolStartPayload>() {
            (
                CorrelationFamily::Tool,
                CorrelationRole::Begin,
                CorrelationIdentity::ToolCall(
                    payload.tool_call_correlation.as_ref()?.tool_call_id.clone(),
                ),
                "apxm.tool.execution",
                GENAI_OPERATION_EXECUTE_TOOL,
                SpanKind::Internal,
            )
        } else {
            let payload = event.payload.downcast_ref::<ToolEndPayload>()?;
            (
                CorrelationFamily::Tool,
                CorrelationRole::End,
                CorrelationIdentity::ToolCall(
                    payload.tool_call_correlation.as_ref()?.tool_call_id.clone(),
                ),
                "apxm.tool.execution",
                GENAI_OPERATION_EXECUTE_TOOL,
                SpanKind::Internal,
            )
        };

    let parent_span_id = if matches!(
        family,
        CorrelationFamily::ModelStep
            | CorrelationFamily::Inference
            | CorrelationFamily::AgentTool
            | CorrelationFamily::Tool
    ) {
        None
    } else {
        event.meta.parent_span_id.clone()
    };

    Some(CorrelationDescriptor {
        key: CorrelationKey {
            family,
            trace_id: event.meta.trace_id.clone(),
            parent_span_id,
            identity,
        },
        role,
        span_name,
        operation,
        span_kind,
    })
}

fn operation_identity(event: &ApxmEvent) -> Option<(u64, String)> {
    let payload = event.payload.to_json();
    let payload = payload.as_object()?;
    let node_id = payload.get("node_id")?.as_u64()?;
    let op_type = payload.get("op_type")?.as_str()?.to_owned();
    Some((node_id, op_type))
}

fn export_emission(tracer: &Tracer, config: GenAiExporterConfig, emission: SpanEmission) {
    match emission {
        SpanEmission::Paired { begin, end } => {
            export_paired_span(tracer, config, begin, *end);
        }
        SpanEmission::Unmatched { pending, outcome } => {
            export_instant_span(
                tracer,
                config,
                pending.event,
                Some((pending.descriptor, outcome)),
            );
        }
        SpanEmission::Instant(event) => export_instant_span(tracer, config, event, None),
    }
}

fn export_paired_span(
    tracer: &Tracer,
    config: GenAiExporterConfig,
    begin: PendingCorrelation,
    end: PendingCorrelation,
) {
    let mut attributes = Vec::with_capacity(32);
    add_common_attributes(&mut attributes, &begin.event, begin.descriptor.operation);
    add_payload_attributes(&mut attributes, &begin.event, config);
    add_payload_attributes(&mut attributes, &end.event, config);
    add_paired_event_attributes(&mut attributes, &begin.event, &end.event);

    let trace_id = derive_trace_id(&begin.event.meta.trace_id);
    let parent_context = parent_context(&begin.event, trace_id);
    let start_time: SystemTime = begin.event.meta.timestamp.into();
    let end_time: SystemTime = end.event.meta.timestamp.into();
    let mut span = tracer
        .span_builder(begin.descriptor.span_name)
        .with_trace_id(trace_id)
        .with_span_id(derive_span_id(&begin.event.meta.span_id))
        .with_kind(begin.descriptor.span_kind)
        .with_start_time(start_time)
        .with_attributes(attributes)
        .start_with_context(tracer, &parent_context);
    span.end_with_timestamp(end_time);
}

fn export_instant_span(
    tracer: &Tracer,
    config: GenAiExporterConfig,
    event: ApxmEvent,
    unmatched: Option<(CorrelationDescriptor, CorrelationOutcome)>,
) {
    let (span_name, operation, span_kind) = match &unmatched {
        Some((descriptor, _)) => (
            format!("{}.unmatched", descriptor.span_name),
            descriptor.operation,
            descriptor.span_kind.clone(),
        ),
        None => {
            let operation = operation_name(event.kind());
            (
                format!("apxm.{}", event.kind().name()),
                operation,
                span_kind(operation),
            )
        }
    };
    let mut attributes = Vec::with_capacity(20);
    add_common_attributes(&mut attributes, &event, operation);
    add_payload_attributes(&mut attributes, &event, config);
    if let Some((descriptor, outcome)) = unmatched {
        attributes.push(KeyValue::new("apxm.span.lifecycle", "unmatched"));
        attributes.push(KeyValue::new("apxm.correlation.outcome", outcome.as_str()));
        attributes.push(KeyValue::new(
            "apxm.correlation.role",
            match descriptor.role {
                CorrelationRole::Begin => "begin",
                CorrelationRole::End => "end",
            },
        ));
    } else {
        attributes.push(KeyValue::new("apxm.span.lifecycle", "instant"));
    }

    let trace_id = derive_trace_id(&event.meta.trace_id);
    let parent_context = parent_context(&event, trace_id);
    let timestamp: SystemTime = event.meta.timestamp.into();
    let mut span = tracer
        .span_builder(span_name)
        .with_trace_id(trace_id)
        .with_span_id(derive_span_id(&event.meta.span_id))
        .with_kind(span_kind)
        .with_start_time(timestamp)
        .with_attributes(attributes)
        .start_with_context(tracer, &parent_context);
    span.end_with_timestamp(timestamp);
}

fn add_paired_event_attributes(attributes: &mut Vec<KeyValue>, begin: &ApxmEvent, end: &ApxmEvent) {
    attributes.push(KeyValue::new("apxm.span.lifecycle", "paired"));
    attributes.push(KeyValue::new("apxm.event.start.kind", begin.kind().name()));
    attributes.push(KeyValue::new(
        "apxm.event.start.seq",
        i64::try_from(begin.meta.seq).unwrap_or(i64::MAX),
    ));
    attributes.push(KeyValue::new(
        "apxm.event.start.span_id",
        begin.meta.span_id.clone(),
    ));
    attributes.push(KeyValue::new("apxm.event.end.kind", end.kind().name()));
    attributes.push(KeyValue::new(
        "apxm.event.end.seq",
        i64::try_from(end.meta.seq).unwrap_or(i64::MAX),
    ));
    attributes.push(KeyValue::new(
        "apxm.event.end.span_id",
        end.meta.span_id.clone(),
    ));
}

/// Map an APXM event kind to its pinned GenAI operation vocabulary.
pub fn operation_name(kind: EventKind) -> &'static str {
    match kind.name() {
        "llm_done"
        | "llm_prompt"
        | "token"
        | "thought"
        | "tool_call"
        | "usage"
        | "llm_step_completed"
        | "subagent_llm_call_begin"
        | "subagent_llm_call_end" => GENAI_OPERATION_CHAT,
        "tool_start" | "tool_end" | "tool_call_begin" | "tool_call_end" => {
            GENAI_OPERATION_EXECUTE_TOOL
        }
        "subagent_spawn_begin" | "subagent_spawn_end" => GENAI_OPERATION_CREATE_AGENT,
        "subagent_done" | "subagent_failed" | "agent_message" | "approval_request"
        | "approval_resolved" => GENAI_OPERATION_INVOKE_AGENT,
        "operation_start"
        | "operation_end"
        | "workflow_started"
        | "workflow_step_started"
        | "workflow_step_completed"
        | "workflow_finished"
        | "execution_started"
        | "execute_complete" => GENAI_OPERATION_INVOKE_WORKFLOW,
        _ => GENAI_OPERATION_EVENT,
    }
}

fn span_kind(operation: &str) -> SpanKind {
    match operation {
        GENAI_OPERATION_CHAT | GENAI_OPERATION_EXECUTE_TOOL => SpanKind::Client,
        _ => SpanKind::Internal,
    }
}

fn parent_context(event: &ApxmEvent, trace_id: TraceId) -> Context {
    match &event.meta.parent_span_id {
        Some(parent_span_id) => Context::new().with_remote_span_context(SpanContext::new(
            trace_id,
            derive_span_id(parent_span_id),
            TraceFlags::SAMPLED,
            true,
            TraceState::default(),
        )),
        None => Context::new(),
    }
}

/// Derive a valid OTel trace id from the event bus's opaque trace string.
pub fn derive_trace_id(value: &str) -> TraceId {
    let mut hasher = Hasher::new();
    hasher.update(b"apxm-otel-trace-id\0");
    hasher.update(value.as_bytes());
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&hasher.finalize().as_bytes()[..16]);
    ensure_nonzero(&mut bytes);
    TraceId::from_bytes(bytes)
}

/// Derive a valid OTel span id from the event bus's opaque span string.
pub fn derive_span_id(value: &str) -> SpanId {
    let mut hasher = Hasher::new();
    hasher.update(b"apxm-otel-span-id\0");
    hasher.update(value.as_bytes());
    let mut bytes = [0_u8; 8];
    bytes.copy_from_slice(&hasher.finalize().as_bytes()[..8]);
    ensure_nonzero(&mut bytes);
    SpanId::from_bytes(bytes)
}

fn ensure_nonzero(bytes: &mut [u8]) {
    if bytes.iter().all(|byte| *byte == 0) {
        let last = bytes.len() - 1;
        bytes[last] = 1;
    }
}

fn add_common_attributes(
    attributes: &mut Vec<KeyValue>,
    event: &ApxmEvent,
    operation: &'static str,
) {
    attributes.push(KeyValue::new("gen_ai.operation.name", operation));
    attributes.push(KeyValue::new("apxm.event.kind", event.kind().name()));
    attributes.push(KeyValue::new(
        "apxm.event.seq",
        i64::try_from(event.meta.seq).unwrap_or(i64::MAX),
    ));
    attributes.push(KeyValue::new(
        "apxm.event.trace_id",
        event.meta.trace_id.clone(),
    ));
    attributes.push(KeyValue::new(
        "apxm.event.span_id",
        event.meta.span_id.clone(),
    ));
    attributes.push(KeyValue::new("apxm.event.id", event.meta.span_id.clone()));
    if let Some(parent_span_id) = &event.meta.parent_span_id {
        attributes.push(KeyValue::new(
            "apxm.event.parent_span_id",
            parent_span_id.clone(),
        ));
    }
    attributes.push(KeyValue::new(
        "apxm.event.source",
        source_name(&event.meta.source),
    ));
    if let Some(scope_id) = &event.meta.scope_id {
        attributes.push(KeyValue::new("apxm.event.scope_id", scope_id.clone()));
    }
    if let Some(program_package) = &event.meta.program_package {
        attributes.push(KeyValue::new(
            "apxm.program_package.id",
            program_package.program_package_id.clone(),
        ));
        attributes.push(KeyValue::new(
            "apxm.program_package.digest",
            program_package.program_package_digest.clone(),
        ));
    }
}

fn source_name(source: &EventSource) -> String {
    match source {
        EventSource::Backend(name) | EventSource::Acp(name) => name.clone(),
        EventSource::Runtime => "runtime".to_string(),
        EventSource::Session => "session".to_string(),
        EventSource::Server => "server".to_string(),
        EventSource::Gui => "gui".to_string(),
    }
}

fn add_generation_identity_attributes(
    attributes: &mut Vec<KeyValue>,
    generation: &GenerationIdentity,
) {
    attributes.push(KeyValue::new(
        "apxm.generation.call_id",
        generation.call_id.clone(),
    ));
    attributes.push(KeyValue::new(
        "apxm.generation.attempt",
        i64::try_from(generation.attempt).unwrap_or(i64::MAX),
    ));
    attributes.push(KeyValue::new(
        "apxm.generation.step_number",
        i64::try_from(generation.step_number).unwrap_or(i64::MAX),
    ));
}

fn add_tool_correlation_attributes(
    attributes: &mut Vec<KeyValue>,
    correlation: &apxm_core::events::payload::ToolCallCorrelation,
) {
    attributes.push(KeyValue::new(
        "gen_ai.tool.call.id",
        correlation.tool_call_id.clone(),
    ));
    add_generation_identity_attributes(attributes, &correlation.generation);
}

fn add_payload_attributes(
    attributes: &mut Vec<KeyValue>,
    event: &ApxmEvent,
    config: GenAiExporterConfig,
) {
    if let Some(payload) = event.payload.downcast_ref::<LlmDonePayload>() {
        if let Some(generation) = &payload.generation {
            add_generation_identity_attributes(attributes, generation);
        }
        attributes.push(KeyValue::new("gen_ai.request.model", payload.model.clone()));
        attributes.push(KeyValue::new(
            "gen_ai.response.finish_reason",
            payload.finish_reason.reason.clone(),
        ));
        attributes.push(KeyValue::new(
            "gen_ai.usage.input_tokens",
            i64::try_from(payload.usage.input_tokens).unwrap_or(i64::MAX),
        ));
        attributes.push(KeyValue::new(
            "gen_ai.usage.output_tokens",
            i64::try_from(payload.usage.output_tokens).unwrap_or(i64::MAX),
        ));
        if let Some(response_id) = &payload.response_id {
            attributes.push(KeyValue::new("gen_ai.response.id", response_id.clone()));
        }
        add_text_attribute(
            attributes,
            "gen_ai.completion",
            "gen_ai.completion.content_type",
            &payload.content,
            config.capture_message_content,
        );
        for tool_call in &payload.tool_calls {
            add_tool_call_attributes(attributes, tool_call, config);
        }
    }

    if let Some(payload) = event.payload.downcast_ref::<LlmStepCompletedPayload>() {
        if let Some(generation) = &payload.generation {
            add_generation_identity_attributes(attributes, generation);
        }
        attributes.push(KeyValue::new("gen_ai.request.model", payload.model.clone()));
        attributes.push(KeyValue::new(
            "gen_ai.response.finish_reason",
            payload.finish_reason.reason.clone(),
        ));
        attributes.push(KeyValue::new(
            "gen_ai.usage.input_tokens",
            i64::try_from(payload.usage.input_tokens).unwrap_or(i64::MAX),
        ));
        attributes.push(KeyValue::new(
            "gen_ai.usage.output_tokens",
            i64::try_from(payload.usage.output_tokens).unwrap_or(i64::MAX),
        ));
        attributes.push(KeyValue::new(
            "apxm.usage.cached_input_tokens",
            i64::try_from(payload.usage.cached_input_tokens).unwrap_or(i64::MAX),
        ));
        attributes.push(KeyValue::new(
            "apxm.usage.reasoning_output_tokens",
            i64::try_from(payload.usage.reasoning_output_tokens).unwrap_or(i64::MAX),
        ));
        attributes.push(KeyValue::new(
            "apxm.llm.step.number",
            i64::try_from(payload.step_number).unwrap_or(i64::MAX),
        ));
        attributes.push(KeyValue::new(
            "apxm.node.id",
            i64::try_from(payload.node_id).unwrap_or(i64::MAX),
        ));
        attributes.push(KeyValue::new(
            "apxm.llm.performance.latency_ms",
            payload.performance.latency_ms,
        ));
        attributes.push(KeyValue::new(
            "apxm.llm.performance.prefill_ms",
            payload.performance.prefill_ms,
        ));
        attributes.push(KeyValue::new(
            "apxm.llm.performance.decode_ms",
            payload.performance.decode_ms,
        ));
        attributes.push(KeyValue::new(
            "apxm.llm.tool_call_count",
            i64::try_from(payload.tool_call_count).unwrap_or(i64::MAX),
        ));
    }

    if let Some(payload) = event.payload.downcast_ref::<TokenUsagePayload>() {
        if let Some(generation) = &payload.generation {
            add_generation_identity_attributes(attributes, generation);
        }
        attributes.push(KeyValue::new(
            "apxm.node.id",
            i64::try_from(payload.node_id).unwrap_or(i64::MAX),
        ));
        attributes.push(KeyValue::new(
            "gen_ai.usage.input_tokens",
            i64::try_from(payload.input_tokens).unwrap_or(i64::MAX),
        ));
        attributes.push(KeyValue::new(
            "gen_ai.usage.output_tokens",
            i64::try_from(payload.output_tokens).unwrap_or(i64::MAX),
        ));
    }

    if let Some(payload) = event.payload.downcast_ref::<ToolCallPayload>() {
        add_tool_call_attributes(attributes, payload, config);
        if let Some(correlation) = &payload.tool_call_correlation {
            add_tool_correlation_attributes(attributes, correlation);
        }
    }

    if let Some(payload) = event.payload.downcast_ref::<LlmPromptPayload>() {
        if let Some(generation) = &payload.generation {
            add_generation_identity_attributes(attributes, generation);
        }
        add_existing_redacted_content(
            attributes,
            "gen_ai.prompt",
            "gen_ai.prompt.content_type",
            &payload.prompt,
        );
    }

    if let Some(payload) = event.payload.downcast_ref::<NodeOutputPayload>() {
        add_existing_redacted_content(
            attributes,
            "apxm.node.output",
            "apxm.node.output.content_type",
            &payload.output,
        );
        attributes.push(KeyValue::new(
            "apxm.node.id",
            i64::try_from(payload.node_id).unwrap_or(i64::MAX),
        ));
        if let Some(node_name) = &payload.node_name {
            attributes.push(KeyValue::new("apxm.node.name", node_name.clone()));
        }
    }

    add_operation_attributes(attributes, event);

    if let Some(payload) = event.payload.downcast_ref::<ToolStartPayload>() {
        if let Some(correlation) = &payload.tool_call_correlation {
            add_tool_correlation_attributes(attributes, correlation);
        }
        attributes.push(KeyValue::new("gen_ai.tool.name", payload.name.clone()));
        attributes.push(KeyValue::new(
            "gen_ai.tool.argument_count",
            i64::try_from(payload.args.len()).unwrap_or(i64::MAX),
        ));
    }

    if let Some(payload) = event.payload.downcast_ref::<ToolEndPayload>() {
        if let Some(correlation) = &payload.tool_call_correlation {
            add_tool_correlation_attributes(attributes, correlation);
        }
        attributes.push(KeyValue::new("gen_ai.tool.name", payload.name.clone()));
        add_json_attribute(
            attributes,
            "gen_ai.tool.result",
            "gen_ai.tool.result.content_type",
            &payload.result,
            config.capture_message_content,
        );
    }

    if let Some(payload) = event.payload.downcast_ref::<SubagentSpawnBeginPayload>() {
        attributes.push(KeyValue::new(
            "gen_ai.agent.name",
            payload.agent_code.clone(),
        ));
        if let Some(agent_type) = &payload.agent_type {
            attributes.push(KeyValue::new("gen_ai.agent.type", agent_type.clone()));
        }
    }

    if let Some(payload) = event.payload.downcast_ref::<SubagentLlmCallEndPayload>() {
        if let Some(generation) = &payload.generation {
            add_generation_identity_attributes(attributes, generation);
        }
        attributes.push(KeyValue::new(
            "gen_ai.agent.name",
            payload.agent_code.clone(),
        ));
        attributes.push(KeyValue::new(
            "gen_ai.response.finish_reason",
            payload.finish_reason.clone(),
        ));
        attributes.push(KeyValue::new(
            "gen_ai.usage.input_tokens",
            i64::try_from(payload.usage.input_tokens).unwrap_or(i64::MAX),
        ));
        attributes.push(KeyValue::new(
            "gen_ai.usage.output_tokens",
            i64::try_from(payload.usage.output_tokens).unwrap_or(i64::MAX),
        ));
    }

    if let Some(payload) = event.payload.downcast_ref::<SubagentLlmCallBeginPayload>() {
        if let Some(generation) = &payload.generation {
            add_generation_identity_attributes(attributes, generation);
        }
        attributes.push(KeyValue::new(
            "gen_ai.agent.name",
            payload.agent_code.clone(),
        ));
        attributes.push(KeyValue::new("gen_ai.request.model", payload.model.clone()));
        attributes.push(KeyValue::new(
            "gen_ai.provider.name",
            payload.backend.clone(),
        ));
        attributes.push(KeyValue::new(
            "gen_ai.tool.count",
            i64::try_from(payload.tool_manifest_count).unwrap_or(i64::MAX),
        ));
    }

    if let Some(payload) = event.payload.downcast_ref::<ToolCallBeginPayload>() {
        if let Some(correlation) = &payload.tool_call_correlation {
            add_tool_correlation_attributes(attributes, correlation);
        }
        attributes.push(KeyValue::new(
            "gen_ai.agent.name",
            payload.agent_code.clone(),
        ));
        attributes.push(KeyValue::new("gen_ai.tool.name", payload.tool_name.clone()));
        attributes.push(KeyValue::new(
            "gen_ai.tool.argument_keys",
            payload.argument_keys.join(","),
        ));
    }

    if let Some(payload) = event.payload.downcast_ref::<ToolCallEndPayload>() {
        if let Some(correlation) = &payload.tool_call_correlation {
            add_tool_correlation_attributes(attributes, correlation);
        }
        attributes.push(KeyValue::new(
            "gen_ai.agent.name",
            payload.agent_code.clone(),
        ));
        attributes.push(KeyValue::new("gen_ai.tool.name", payload.tool_name.clone()));
        attributes.push(KeyValue::new("gen_ai.tool.status", payload.status.as_str()));
        attributes.push(KeyValue::new(
            "gen_ai.tool.latency_ms",
            i64::try_from(payload.latency_ms).unwrap_or(i64::MAX),
        ));
        attributes.push(KeyValue::new(
            "gen_ai.tool.result_keys",
            payload.result_keys.join(","),
        ));
    }

    if let Some(payload) = event.payload.downcast_ref::<AgentMessagePayload>() {
        add_text_attribute(
            attributes,
            "gen_ai.output.messages",
            "gen_ai.output.messages.content_type",
            &payload.text,
            config.capture_message_content,
        );
        if let Some(response_id) = &payload.response_id {
            attributes.push(KeyValue::new("gen_ai.response.id", response_id.clone()));
        }
    }

    if let Some(payload) = event.payload.downcast_ref::<TokenPayload>() {
        add_text_attribute(
            attributes,
            "gen_ai.event.content",
            "gen_ai.event.content_type",
            &payload.text,
            config.capture_message_content,
        );
    }

    if let Some(payload) = event.payload.downcast_ref::<ThoughtPayload>() {
        add_text_attribute(
            attributes,
            "gen_ai.event.content",
            "gen_ai.event.content_type",
            &payload.text,
            config.capture_message_content,
        );
        if let Some(summary) = &payload.summary {
            attributes.push(KeyValue::new("gen_ai.event.summary", summary.clone()));
        }
    }

    if let Some(payload) = event.payload.downcast_ref::<ExecuteCompletePayload>() {
        add_json_attribute(
            attributes,
            "gen_ai.output.result",
            "gen_ai.output.result.content_type",
            &payload.result,
            config.capture_message_content,
        );
    }
}

fn add_operation_attributes(attributes: &mut Vec<KeyValue>, event: &ApxmEvent) {
    if !matches!(event.kind().name(), "operation_start" | "operation_end") {
        return;
    }

    let payload = event.payload.to_json();
    let Some(payload) = payload.as_object() else {
        return;
    };

    if let Some(operation) = payload.get("op_type").and_then(serde_json::Value::as_str) {
        attributes.push(KeyValue::new("apxm.operation.type", operation.to_owned()));
    }
    if let Some(node_id) = payload.get("node_id").and_then(serde_json::Value::as_u64) {
        attributes.push(KeyValue::new(
            "apxm.node.id",
            i64::try_from(node_id).unwrap_or(i64::MAX),
        ));
    }
    if event.kind().name() == "operation_end" {
        if let Some(success) = payload.get("success").and_then(serde_json::Value::as_bool) {
            attributes.push(KeyValue::new("apxm.operation.success", success));
        }
        if let Some(duration_ms) = payload
            .get("duration_ms")
            .and_then(serde_json::Value::as_u64)
        {
            attributes.push(KeyValue::new(
                "apxm.operation.duration_ms",
                i64::try_from(duration_ms).unwrap_or(i64::MAX),
            ));
        }
    }
}

fn add_tool_call_attributes(
    attributes: &mut Vec<KeyValue>,
    payload: &ToolCallPayload,
    config: GenAiExporterConfig,
) {
    attributes.push(KeyValue::new("gen_ai.tool.name", payload.name.clone()));
    attributes.push(KeyValue::new("gen_ai.tool.call.id", payload.id.clone()));
    add_json_attribute(
        attributes,
        "gen_ai.tool.arguments",
        "gen_ai.tool.arguments.content_type",
        &payload.arguments,
        config.capture_message_content,
    );
}

fn add_text_attribute(
    attributes: &mut Vec<KeyValue>,
    value_key: &'static str,
    content_type_key: &'static str,
    value: &str,
    capture: bool,
) {
    if capture {
        attributes.push(KeyValue::new(value_key, value.to_string()));
        attributes.push(KeyValue::new(content_type_key, "text/plain"));
    } else {
        let redacted = RedactedContent::from_text(value);
        add_redacted_content(attributes, value_key, content_type_key, &redacted);
    }
}

fn add_json_attribute(
    attributes: &mut Vec<KeyValue>,
    value_key: &'static str,
    content_type_key: &'static str,
    value: &serde_json::Value,
    capture: bool,
) {
    if capture {
        attributes.push(KeyValue::new(value_key, value.to_string()));
        attributes.push(KeyValue::new(content_type_key, "application/json"));
    } else {
        let redacted = RedactedContent::from_json(value);
        add_redacted_content(attributes, value_key, content_type_key, &redacted);
    }
}

fn add_redacted_content(
    attributes: &mut Vec<KeyValue>,
    value_key: &'static str,
    content_type_key: &'static str,
    value: &RedactedContent,
) {
    attributes.push(KeyValue::new(value_key, value.hash.clone()));
    attributes.push(KeyValue::new(content_type_key, GENAI_REDACTED_CONTENT_TYPE));
    attributes.push(KeyValue::new("apxm.content.redacted", true));
    attributes.push(KeyValue::new(
        "apxm.content.size_bytes",
        i64::try_from(value.size_bytes).unwrap_or(i64::MAX),
    ));
}

fn add_existing_redacted_content(
    attributes: &mut Vec<KeyValue>,
    value_key: &'static str,
    content_type_key: &'static str,
    value: &RedactedContent,
) {
    attributes.push(KeyValue::new(value_key, value.hash.clone()));
    attributes.push(KeyValue::new(content_type_key, value.content_type.clone()));
    attributes.push(KeyValue::new("apxm.content.redacted", value.redacted));
    attributes.push(KeyValue::new(
        "apxm.content.size_bytes",
        i64::try_from(value.size_bytes).unwrap_or(i64::MAX),
    ));
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::sync::{Arc, Mutex};

    use apxm_core::events::kind::CORE_EVENT_KINDS;
    use apxm_core::events::payload::{
        EventPayload, FinishReasonPayload, LlmStepPerformancePayload, LlmStepUsagePayload,
        ToolCallCorrelation, ToolCallStatus, UnknownEventPayload, UsagePayload,
    };
    use apxm_core::events::{ApxmEvent, EventSource};
    use futures_util::future::BoxFuture;
    use opentelemetry::Value;
    use opentelemetry::trace::{SpanId, TraceId};
    use opentelemetry_sdk::Resource;
    use opentelemetry_sdk::export::trace::{ExportResult, SpanData, SpanExporter};
    use opentelemetry_sdk::testing::trace::InMemorySpanExporter;
    use opentelemetry_sdk::trace::TracerProvider;
    use uuid::Uuid;

    use super::*;

    #[derive(Clone, Debug, Default)]
    struct ResourceCapturingExporter {
        resource: Arc<Mutex<Option<Resource>>>,
    }

    impl SpanExporter for ResourceCapturingExporter {
        fn export(&mut self, _batch: Vec<SpanData>) -> BoxFuture<'static, ExportResult> {
            Box::pin(std::future::ready(Ok(())))
        }

        fn set_resource(&mut self, resource: &Resource) {
            *self.resource.lock().expect("resource lock poisoned") = Some(resource.clone());
        }
    }

    fn test_exporter(
        config: GenAiExporterConfig,
    ) -> (
        GenAiEventExporter,
        InMemorySpanExporter,
        Arc<Mutex<Option<Resource>>>,
    ) {
        test_exporter_with_limit(config, MAX_PENDING_CORRELATIONS)
    }

    fn test_exporter_with_limit(
        config: GenAiExporterConfig,
        max_pending: usize,
    ) -> (
        GenAiEventExporter,
        InMemorySpanExporter,
        Arc<Mutex<Option<Resource>>>,
    ) {
        let spans = InMemorySpanExporter::default();
        let resource_capture = Arc::new(Mutex::new(None));
        let provider = TracerProvider::builder()
            .with_simple_exporter(spans.clone())
            .with_simple_exporter(ResourceCapturingExporter {
                resource: resource_capture.clone(),
            })
            .with_resource(genai_resource("apxm-test"))
            .build();
        let exporter = GenAiEventExporter::new_with_max_pending(&provider, config, max_pending);
        (exporter, spans, resource_capture)
    }

    fn event(payload: impl EventPayload) -> ApxmEvent {
        ApxmEvent::root(payload, EventSource::Runtime, "trace-1").with_seq(7)
    }

    fn correlated_event(
        payload: impl EventPayload,
        span_id: &str,
        parent_span_id: &str,
        seq: u64,
    ) -> ApxmEvent {
        ApxmEvent::child_of(payload, EventSource::Runtime, "trace-1", parent_span_id)
            .with_span_id(span_id)
            .with_seq(seq)
    }

    fn string_values(spans: &[SpanData]) -> Vec<String> {
        spans
            .iter()
            .flat_map(|span| span.attributes.iter())
            .filter_map(|attribute| match &attribute.value {
                Value::String(value) => Some(value.as_str().to_owned()),
                _ => None,
            })
            .collect()
    }

    fn attribute_value<'a>(span: &'a SpanData, key: &str) -> Option<&'a Value> {
        span.attributes
            .iter()
            .find(|attribute| attribute.key.as_str() == key)
            .map(|attribute| &attribute.value)
    }

    #[test]
    fn schema_pin_is_present_on_resource_scope_and_every_exported_span() {
        let (exporter, spans, resource_capture) = test_exporter(GenAiExporterConfig::enabled());
        exporter.emit(event(UnknownEventPayload::from_json(
            "future_event",
            serde_json::json!({"safe": true}),
        )));

        let resource = resource_capture
            .lock()
            .expect("resource lock poisoned")
            .clone()
            .expect("provider resource was not propagated");
        assert_eq!(resource.schema_url(), Some(GENAI_SEMCONV_SCHEMA_URL));

        let spans = spans.get_finished_spans().expect("span export failed");
        assert!(!spans.is_empty());
        for span in spans {
            assert_eq!(
                span.instrumentation_scope.schema_url(),
                Some(GENAI_SEMCONV_SCHEMA_URL)
            );
        }
    }

    #[test]
    fn representative_event_kinds_map_to_genai_operations() {
        let (exporter, spans, _) = test_exporter(GenAiExporterConfig::enabled());
        exporter.emit(event(UnknownEventPayload::from_json(
            "operation_start",
            serde_json::json!({}),
        )));
        exporter.emit(event(LlmDonePayload {
            content: "done".to_string(),
            model: "model".to_string(),
            finish_reason: FinishReasonPayload {
                reason: "stop".to_string(),
            },
            usage: UsagePayload {
                input_tokens: 1,
                output_tokens: 2,
                generation: None,
            },
            tool_calls: Vec::new(),
            response_id: None,
            generation: None,
        }));
        exporter.emit(event(ToolStartPayload {
            name: "lookup".to_string(),
            args: std::collections::HashMap::new(),
            tool_call_correlation: None,
        }));
        exporter.emit(event(AgentMessagePayload {
            text: "done".to_string(),
            item_id: None,
            response_id: None,
            usage: None,
        }));
        exporter.emit(event(SubagentSpawnBeginPayload {
            agent_code: "worker".to_string(),
            agent_name: None,
            agent_type: None,
            module_key: None,
            autonomy_policy: None,
            parent_span_id: None,
        }));
        exporter.flush_pending();

        let spans = spans.get_finished_spans().expect("span export failed");
        let operations: HashSet<_> = spans
            .iter()
            .filter_map(|span| attribute_value(span, "gen_ai.operation.name"))
            .map(Value::as_str)
            .map(|value| value.into_owned())
            .collect();
        assert_eq!(
            operations,
            HashSet::from([
                GENAI_OPERATION_INVOKE_WORKFLOW.to_string(),
                GENAI_OPERATION_CHAT.to_string(),
                GENAI_OPERATION_EXECUTE_TOOL.to_string(),
                GENAI_OPERATION_INVOKE_AGENT.to_string(),
                GENAI_OPERATION_CREATE_AGENT.to_string(),
            ])
        );
    }

    #[test]
    fn llm_step_exports_typed_observability_attributes() {
        let (exporter, spans, _) = test_exporter(GenAiExporterConfig::enabled());
        exporter.emit(event(LlmStepCompletedPayload {
            node_id: 7,
            step_number: 2,
            model: "amd/model".to_string(),
            finish_reason: FinishReasonPayload {
                reason: "tool_use".to_string(),
            },
            usage: apxm_core::events::payload::LlmStepUsagePayload {
                input_tokens: 120,
                output_tokens: 24,
                cached_input_tokens: 32,
                reasoning_output_tokens: 8,
            },
            performance: apxm_core::events::payload::LlmStepPerformancePayload {
                latency_ms: 245.5,
                prefill_ms: 80.25,
                decode_ms: 165.25,
            },
            tool_call_count: 2,
            generation: None,
        }));
        let spans = spans.get_finished_spans().expect("span export failed");
        assert_eq!(spans.len(), 1);
        assert_eq!(
            attribute_value(&spans[0], "gen_ai.operation.name").map(Value::as_str),
            Some(std::borrow::Cow::Borrowed(GENAI_OPERATION_CHAT))
        );
        assert_eq!(
            attribute_value(&spans[0], "apxm.llm.step.number"),
            Some(&Value::I64(2))
        );
        assert_eq!(
            attribute_value(&spans[0], "apxm.llm.performance.latency_ms"),
            Some(&Value::F64(245.5))
        );
        assert_eq!(
            attribute_value(&spans[0], "apxm.usage.cached_input_tokens"),
            Some(&Value::I64(32))
        );
    }

    #[test]
    fn token_usage_exports_generation_identity_and_counts() {
        let (exporter, spans, _) = test_exporter(GenAiExporterConfig::enabled());
        exporter.emit(event(TokenUsagePayload {
            node_id: 7,
            input_tokens: 11,
            output_tokens: 13,
            generation: Some(GenerationIdentity::new("call-usage", 2, 4)),
        }));

        let spans = spans.get_finished_spans().expect("span export failed");
        assert_eq!(spans.len(), 1);
        let span = &spans[0];
        assert_eq!(
            attribute_value(span, "apxm.generation.call_id").map(Value::as_str),
            Some(std::borrow::Cow::Borrowed("call-usage"))
        );
        assert_eq!(
            attribute_value(span, "apxm.generation.attempt"),
            Some(&Value::I64(2))
        );
        assert_eq!(
            attribute_value(span, "apxm.generation.step_number"),
            Some(&Value::I64(4))
        );
        assert_eq!(attribute_value(span, "apxm.node.id"), Some(&Value::I64(7)));
        assert_eq!(
            attribute_value(span, "gen_ai.usage.input_tokens"),
            Some(&Value::I64(11))
        );
        assert_eq!(
            attribute_value(span, "gen_ai.usage.output_tokens"),
            Some(&Value::I64(13))
        );
    }

    #[test]
    fn paired_events_form_one_duration_bearing_span_with_stable_ids() {
        let (exporter, spans, _) = test_exporter(GenAiExporterConfig::enabled());
        let correlation =
            ToolCallCorrelation::new(GenerationIdentity::new("generation-1", 1, 1), "tool-call-1");
        let begin = correlated_event(
            ToolCallBeginPayload {
                agent_code: "worker".to_string(),
                tool_name: "lookup".to_string(),
                argument_keys: Vec::new(),
                tool_call_correlation: Some(correlation.clone()),
            },
            "pair-begin",
            "run-parent",
            10,
        );
        exporter.emit(begin.clone());
        assert!(
            spans
                .get_finished_spans()
                .expect("span export failed")
                .is_empty()
        );

        std::thread::sleep(std::time::Duration::from_millis(2));
        let end = correlated_event(
            ToolCallEndPayload {
                agent_code: "worker".to_string(),
                tool_name: "lookup".to_string(),
                result_keys: Vec::new(),
                status: ToolCallStatus::Ok,
                latency_ms: 2,
                tool_call_correlation: Some(correlation),
            },
            "pair-end",
            "run-parent",
            11,
        );
        exporter.emit(end.clone());

        let spans = spans.get_finished_spans().expect("span export failed");
        assert_eq!(spans.len(), 1);
        let span = &spans[0];
        assert_eq!(span.name.as_ref(), "apxm.tool.operation");
        assert!(span.end_time > span.start_time);
        assert_eq!(
            span.span_context.span_id(),
            derive_span_id(&begin.meta.span_id)
        );
        assert_eq!(span.parent_span_id, derive_span_id("run-parent"));
        assert_eq!(
            attribute_value(span, "apxm.span.lifecycle").map(Value::as_str),
            Some(std::borrow::Cow::Borrowed("paired"))
        );
        assert_eq!(
            attribute_value(span, "apxm.event.start.span_id").map(Value::as_str),
            Some(std::borrow::Cow::Borrowed("pair-begin"))
        );
        assert_eq!(
            attribute_value(span, "apxm.event.end.span_id").map(Value::as_str),
            Some(std::borrow::Cow::Borrowed("pair-end"))
        );
    }

    #[test]
    fn model_and_tool_pairs_correlate_when_terminals_arrive_first() {
        let (exporter, spans, _) = test_exporter(GenAiExporterConfig::enabled());
        let generation = GenerationIdentity::new("generation-1", 1, 1);
        let tool_correlation = ToolCallCorrelation::new(generation.clone(), "tool-call-1");
        let model_begin = correlated_event(
            SubagentLlmCallBeginPayload {
                agent_code: "researcher".to_string(),
                model: "model-1".to_string(),
                backend: "gateway".to_string(),
                tool_manifest_count: 2,
                generation: Some(generation.clone()),
            },
            "model-begin",
            "model-parent",
            20,
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
        let model_end = correlated_event(
            SubagentLlmCallEndPayload {
                agent_code: "researcher".to_string(),
                finish_reason: "stop".to_string(),
                usage: UsagePayload {
                    input_tokens: 12,
                    output_tokens: 4,
                    generation: None,
                },
                content_len: 32,
                generation: Some(generation.clone()),
            },
            "model-end",
            "model-parent",
            21,
        );
        exporter.emit(model_end);
        exporter.emit(model_begin);

        let inference_begin = correlated_event(
            LlmPromptPayload {
                node_id: 7,
                node_name: Some("answer".to_string()),
                prompt: RedactedContent::from_text("private prompt"),
                generation: Some(generation.clone()),
            },
            "inference-begin",
            "inference-parent",
            25,
        );
        exporter.emit(inference_begin);
        std::thread::sleep(std::time::Duration::from_millis(2));
        exporter.emit(correlated_event(
            LlmStepCompletedPayload {
                node_id: 7,
                step_number: 1,
                model: "model-1".to_string(),
                finish_reason: FinishReasonPayload {
                    reason: "stop".to_string(),
                },
                usage: LlmStepUsagePayload {
                    input_tokens: 12,
                    output_tokens: 4,
                    cached_input_tokens: 0,
                    reasoning_output_tokens: 0,
                },
                performance: LlmStepPerformancePayload {
                    latency_ms: 2.0,
                    prefill_ms: 0.0,
                    decode_ms: 0.0,
                },
                tool_call_count: 0,
                generation: Some(generation.clone()),
            },
            "inference-end",
            "inference-parent",
            26,
        ));

        let tool_begin = correlated_event(
            ToolCallBeginPayload {
                agent_code: "researcher".to_string(),
                tool_name: "lookup".to_string(),
                argument_keys: vec!["query".to_string()],
                tool_call_correlation: Some(tool_correlation.clone()),
            },
            "tool-begin",
            "tool-parent",
            30,
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
        let tool_end = correlated_event(
            ToolCallEndPayload {
                agent_code: "researcher".to_string(),
                tool_name: "lookup".to_string(),
                result_keys: vec!["items".to_string()],
                status: ToolCallStatus::Ok,
                latency_ms: 2,
                tool_call_correlation: Some(tool_correlation),
            },
            "tool-end",
            "tool-parent",
            31,
        );
        exporter.emit(tool_end);
        exporter.emit(tool_begin);

        let operation_begin = correlated_event(
            UnknownEventPayload::from_json(
                "operation_start",
                serde_json::json!({"node_id": 7, "op_type": "ASK"}),
            ),
            "operation-begin",
            "operation-parent",
            40,
        );
        exporter.emit(operation_begin);
        std::thread::sleep(std::time::Duration::from_millis(2));
        exporter.emit(correlated_event(
            UnknownEventPayload::from_json(
                "operation_end",
                serde_json::json!({
                    "node_id": 7,
                    "op_type": "ASK",
                    "success": true,
                    "duration_ms": 2
                }),
            ),
            "operation-end",
            "operation-parent",
            41,
        ));

        let spans = spans.get_finished_spans().expect("span export failed");
        assert_eq!(spans.len(), 4);
        let model = spans
            .iter()
            .find(|span| span.name.as_ref() == "apxm.model.step")
            .expect("model step span missing");
        let inference = spans
            .iter()
            .find(|span| span.name.as_ref() == "apxm.model.inference")
            .expect("model inference span missing");
        let tool = spans
            .iter()
            .find(|span| span.name.as_ref() == "apxm.tool.operation")
            .expect("tool operation span missing");
        let operation = spans
            .iter()
            .find(|span| span.name.as_ref() == "apxm.operation")
            .expect("operation span missing");
        assert!(model.end_time > model.start_time);
        assert!(inference.end_time > inference.start_time);
        assert!(tool.end_time > tool.start_time);
        assert!(operation.end_time > operation.start_time);
        assert_eq!(model.parent_span_id, derive_span_id("model-parent"));
        assert_eq!(inference.parent_span_id, derive_span_id("inference-parent"));
        assert_eq!(tool.parent_span_id, derive_span_id("tool-parent"));
        assert_eq!(operation.parent_span_id, derive_span_id("operation-parent"));
        assert_eq!(
            attribute_value(tool, "gen_ai.tool.status").map(Value::as_str),
            Some(std::borrow::Cow::Borrowed("ok"))
        );
        assert_eq!(
            attribute_value(operation, "apxm.operation.success"),
            Some(&Value::Bool(true))
        );
    }

    #[test]
    fn unmatched_events_are_flushed_as_instantaneous_spans_and_state_is_bounded() {
        let (exporter, spans, _) = test_exporter_with_limit(GenAiExporterConfig::enabled(), 2);
        for index in 0..3 {
            exporter.emit(correlated_event(
                ToolCallBeginPayload {
                    agent_code: "worker".to_string(),
                    tool_name: "lookup".to_string(),
                    argument_keys: Vec::new(),
                    tool_call_correlation: Some(ToolCallCorrelation::new(
                        GenerationIdentity::new(format!("generation-{index}"), 1, 1),
                        format!("tool-call-{index}"),
                    )),
                },
                &format!("begin-{index}"),
                "run-parent",
                index,
            ));
        }

        assert_eq!(exporter.pending_correlation_count(), 2);
        let exported = spans.get_finished_spans().expect("span export failed");
        assert_eq!(exported.len(), 1);
        assert_eq!(exported[0].start_time, exported[0].end_time);
        assert_eq!(
            attribute_value(&exported[0], "apxm.correlation.outcome").map(Value::as_str),
            Some(std::borrow::Cow::Borrowed("capacity_evicted"))
        );

        exporter.flush_pending();
        let exported = spans.get_finished_spans().expect("span export failed");
        assert_eq!(exported.len(), 3);
        assert_eq!(exporter.pending_correlation_count(), 0);
        assert!(exported.iter().all(|span| span.start_time == span.end_time));
        assert!(exported.iter().all(|span| {
            attribute_value(span, "apxm.span.lifecycle").map(Value::as_str)
                == Some(std::borrow::Cow::Borrowed("unmatched"))
        }));
    }

    #[test]
    fn content_capture_is_off_by_default_and_hashes_raw_fields() {
        let (exporter, spans, _) = test_exporter(GenAiExporterConfig::enabled());
        let pii = "alice@example.test SSN 000-12-3456";
        let secret = "sk-test-secret";
        let arguments = serde_json::json!({"token": secret});
        exporter.emit(event(LlmDonePayload {
            content: pii.to_string(),
            model: "model".to_string(),
            finish_reason: FinishReasonPayload {
                reason: "stop".to_string(),
            },
            usage: UsagePayload {
                input_tokens: 1,
                output_tokens: 2,
                generation: None,
            },
            tool_calls: vec![ToolCallPayload {
                id: "call-1".to_string(),
                name: "vault".to_string(),
                arguments: arguments.clone(),
                tool_call_correlation: None,
            }],
            response_id: None,
            generation: None,
        }));
        exporter.flush_pending();

        let spans = spans.get_finished_spans().expect("span export failed");
        let values = string_values(&spans);
        assert!(!values.iter().any(|value| value.contains(pii)));
        assert!(!values.iter().any(|value| value.contains(secret)));
        assert!(values.iter().any(|value| value.starts_with("blake3:")));
        assert!(
            values
                .iter()
                .any(|value| value == GENAI_REDACTED_CONTENT_TYPE)
        );
    }

    #[test]
    fn content_capture_opt_in_preserves_literal_content() {
        let config = GenAiExporterConfig::enabled().with_content_capture(true);
        let (exporter, spans, _) = test_exporter(config);
        let content = "alice@example.test";
        let arguments = serde_json::json!({"token": "sk-test-secret"});
        exporter.emit(event(LlmDonePayload {
            content: content.to_string(),
            model: "model".to_string(),
            finish_reason: FinishReasonPayload {
                reason: "stop".to_string(),
            },
            usage: UsagePayload {
                input_tokens: 1,
                output_tokens: 2,
                generation: None,
            },
            tool_calls: vec![ToolCallPayload {
                id: "call-1".to_string(),
                name: "vault".to_string(),
                arguments: arguments.clone(),
                tool_call_correlation: None,
            }],
            response_id: None,
            generation: None,
        }));
        exporter.flush_pending();

        let spans = spans.get_finished_spans().expect("span export failed");
        let values = string_values(&spans);
        assert!(values.iter().any(|value| value == content));
        assert!(values.iter().any(|value| value == &arguments.to_string()));
    }

    #[test]
    fn default_config_is_disabled_and_disablement_emits_no_spans() {
        let (exporter, spans, _) = test_exporter(GenAiExporterConfig::default());
        assert!(!exporter.is_enabled());
        assert!(!exporter.config().capture_message_content);
        exporter.emit(event(UnknownEventPayload::from_json(
            "disabled_event",
            serde_json::json!({"should_not": "export"}),
        )));
        assert!(
            spans
                .get_finished_spans()
                .expect("span export failed")
                .is_empty()
        );
    }

    #[test]
    fn ids_are_valid_deterministic_collision_resistant_and_nested() {
        let mut trace_ids = HashSet::new();
        let mut span_ids = HashSet::new();
        for _ in 0..1_001 {
            let trace = Uuid::new_v4().to_string();
            let span = Uuid::new_v4().to_string();
            assert!(trace_ids.insert(derive_trace_id(&trace)));
            assert!(span_ids.insert(derive_span_id(&span)));
        }

        let (exporter, spans, _) = test_exporter(GenAiExporterConfig::enabled());
        let root = event(UnknownEventPayload::from_json(
            "root_event",
            serde_json::json!({}),
        ))
        .with_span_id("root-uuid-shaped-span");
        let child = ApxmEvent::child_of(
            UnknownEventPayload::from_json("child_event", serde_json::json!({})),
            EventSource::Runtime,
            root.meta.trace_id.clone(),
            root.meta.span_id.clone(),
        )
        .with_span_id("child-uuid-shaped-span");
        exporter.emit(root.clone());
        exporter.emit(child);

        let spans = spans.get_finished_spans().expect("span export failed");
        assert_eq!(spans.len(), 2);
        assert_eq!(
            spans[0].span_context.trace_id(),
            derive_trace_id(&root.meta.trace_id)
        );
        assert_eq!(
            spans[0].span_context.span_id(),
            derive_span_id(&root.meta.span_id)
        );
        assert_eq!(spans[0].parent_span_id, SpanId::INVALID);
        assert_eq!(
            spans[1].span_context.trace_id(),
            spans[0].span_context.trace_id()
        );
        assert_eq!(spans[1].parent_span_id, spans[0].span_context.span_id());
        assert_ne!(
            spans[1].span_context.span_id(),
            spans[0].span_context.span_id()
        );
        assert_ne!(spans[0].span_context.trace_id(), TraceId::INVALID);
    }

    #[test]
    fn every_registered_kind_and_unknown_payload_is_tolerated() {
        let (exporter, spans, _) = test_exporter(GenAiExporterConfig::enabled());
        for kind in CORE_EVENT_KINDS {
            exporter.emit(event(UnknownEventPayload::from_json(
                kind.name(),
                serde_json::json!({}),
            )));
        }
        exporter.emit(event(UnknownEventPayload::from_json(
            "future_kind",
            serde_json::json!({"new": "field"}),
        )));

        assert_eq!(
            spans
                .get_finished_spans()
                .expect("span export failed")
                .len(),
            CORE_EVENT_KINDS.len() + 1
        );
    }
}
