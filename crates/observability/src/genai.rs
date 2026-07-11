//! Optional OTel GenAI spans emitted from the APXM event bus.
//!
//! The exporter is deliberately an ordinary [`EventEmitter`]. It has no
//! execution authority and no error path back to the runtime: registration is
//! optional, event attributes are bounded to known fields, and raw content is
//! redacted before it can become span data unless an operator opts in.

use std::time::SystemTime;

use apxm_core::events::kind::EventKind;
use apxm_core::events::payload::{
    AgentMessagePayload, ExecuteCompletePayload, LlmDonePayload, LlmPromptPayload,
    LlmStepCompletedPayload, NodeOutputPayload, RedactedContent, SubagentLlmCallEndPayload,
    SubagentSpawnBeginPayload, ThoughtPayload, TokenPayload, ToolCallBeginPayload,
    ToolCallEndPayload, ToolCallPayload, ToolEndPayload, ToolStartPayload, TurnAbortedPayload,
    TurnBoundaryPayload, TurnCompletePayload, TurnDirection, TurnStartedPayload,
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

/// OTel GenAI operation names used by the event-to-span mapping.
pub const GENAI_OPERATION_CHAT: &str = "chat";
pub const GENAI_OPERATION_CREATE_AGENT: &str = "create_agent";
pub const GENAI_OPERATION_EXECUTE_TOOL: &str = "execute_tool";
pub const GENAI_OPERATION_INVOKE_AGENT: &str = "invoke_agent";
pub const GENAI_OPERATION_INVOKE_WORKFLOW: &str = "invoke_workflow";
pub const GENAI_OPERATION_EVENT: &str = "apxm.event";

/// Registration and privacy controls for the GenAI event exporter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GenAiExporterConfig {
    /// Whether the exporter is active. The default is disabled so constructing
    /// the adapter cannot change execution or create telemetry by accident.
    pub enabled: bool,
    /// Whether raw message/tool content may be included in span attributes.
    /// The default is false; hashes and shape metadata remain available.
    pub capture_message_content: bool,
}

impl Default for GenAiExporterConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            capture_message_content: false,
        }
    }
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

/// An optional [`EventEmitter`] that turns each `ApxmEvent` into one OTel span.
///
/// The adapter owns only a named tracer from the caller-provided provider. It
/// does not install a global provider, alter the event, or return export errors
/// to the execution path. Callers can therefore fan it out beside their normal
/// event sink and remove it without changing runtime semantics.
#[derive(Clone, Debug)]
pub struct GenAiEventExporter {
    tracer: Tracer,
    config: GenAiExporterConfig,
}

impl GenAiEventExporter {
    /// Create an exporter from an existing provider.
    ///
    /// The provider should carry [`GENAI_SEMCONV_SCHEMA_URL`] on its resource;
    /// [`genai_resource`] is the canonical helper for that setup. The named
    /// instrumentation scope is also pinned so downstream consumers retain the
    /// schema identity even when they surface scope metadata separately.
    pub fn new(provider: &TracerProvider, config: GenAiExporterConfig) -> Self {
        let scope = InstrumentationScope::builder(GENAI_INSTRUMENTATION_SCOPE)
            .with_version(env!("CARGO_PKG_VERSION"))
            .with_schema_url(GENAI_SEMCONV_SCHEMA_URL)
            .build();

        Self {
            tracer: provider.tracer_with_scope(scope),
            config,
        }
    }

    /// Return the immutable adapter configuration.
    pub const fn config(&self) -> GenAiExporterConfig {
        self.config
    }

    /// Whether this adapter will emit spans.
    pub const fn is_enabled(&self) -> bool {
        self.config.enabled
    }
}

impl EventEmitter for GenAiEventExporter {
    fn emit(&self, event: ApxmEvent) {
        if !self.config.enabled {
            return;
        }

        let operation = operation_name(event.kind());
        let mut attributes = Vec::with_capacity(16);
        add_common_attributes(&mut attributes, &event, operation);
        add_payload_attributes(&mut attributes, &event, self.config);

        let trace_id = derive_trace_id(&event.meta.trace_id);
        let span_id = derive_span_id(&event.meta.span_id);
        let parent_context = parent_context(&event, trace_id);
        let timestamp: SystemTime = event.meta.timestamp.into();

        let mut span = self
            .tracer
            .span_builder(format!("apxm.{}", event.kind().name()))
            .with_trace_id(trace_id)
            .with_span_id(span_id)
            .with_kind(span_kind(operation))
            .with_start_time(timestamp)
            .with_end_time(timestamp)
            .with_attributes(attributes)
            .start_with_context(&self.tracer, &parent_context);
        span.end();
    }
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
        "turn_started" | "turn_complete" | "turn_aborted" | "turn_boundary" | "subagent_done"
        | "subagent_failed" | "agent_message" | "approval_request" | "approval_resolved" => {
            GENAI_OPERATION_INVOKE_AGENT
        }
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
    attributes.push(KeyValue::new(
        "apxm.event.source",
        source_name(&event.meta.source),
    ));
    if let Some(scope_id) = &event.meta.scope_id {
        attributes.push(KeyValue::new("apxm.event.scope_id", scope_id.clone()));
    }
    if let Some(skill) = &event.meta.skill {
        attributes.push(KeyValue::new("apxm.skill.id", skill.skill_id.clone()));
        attributes.push(KeyValue::new(
            "apxm.skill.version",
            skill.skill_version.clone(),
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

fn add_payload_attributes(
    attributes: &mut Vec<KeyValue>,
    event: &ApxmEvent,
    config: GenAiExporterConfig,
) {
    if let Some(payload) = event.payload.downcast_ref::<LlmDonePayload>() {
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

    if let Some(payload) = event.payload.downcast_ref::<ToolCallPayload>() {
        add_tool_call_attributes(attributes, payload, config);
    }

    if let Some(payload) = event.payload.downcast_ref::<LlmPromptPayload>() {
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
        attributes.push(KeyValue::new("gen_ai.tool.name", payload.name.clone()));
        attributes.push(KeyValue::new(
            "gen_ai.tool.argument_count",
            i64::try_from(payload.args.len()).unwrap_or(i64::MAX),
        ));
    }

    if let Some(payload) = event.payload.downcast_ref::<ToolEndPayload>() {
        attributes.push(KeyValue::new("gen_ai.tool.name", payload.name.clone()));
        add_json_attribute(
            attributes,
            "gen_ai.tool.result",
            "gen_ai.tool.result.content_type",
            &payload.result,
            config.capture_message_content,
        );
    }

    if let Some(payload) = event.payload.downcast_ref::<TurnStartedPayload>() {
        attributes.push(KeyValue::new(
            "gen_ai.agent.execution.id",
            payload.execution_id.clone(),
        ));
        if let Some(turn_id) = &payload.turn_id {
            attributes.push(KeyValue::new("gen_ai.agent.turn.id", turn_id.clone()));
        }
    }

    if let Some(payload) = event.payload.downcast_ref::<TurnCompletePayload>() {
        attributes.push(KeyValue::new(
            "gen_ai.agent.execution.id",
            payload.execution_id.clone(),
        ));
        attributes.push(KeyValue::new("gen_ai.agent.had_answer", payload.had_answer));
        attributes.push(KeyValue::new(
            "gen_ai.agent.duration_ms",
            i64::try_from(payload.duration_ms).unwrap_or(i64::MAX),
        ));
    }

    if let Some(payload) = event.payload.downcast_ref::<TurnBoundaryPayload>() {
        attributes.push(KeyValue::new(
            "gen_ai.agent.turn.number",
            i64::try_from(payload.turn_number).unwrap_or(i64::MAX),
        ));
        attributes.push(KeyValue::new(
            "gen_ai.agent.turn.direction",
            match payload.direction {
                TurnDirection::Request => "request",
                TurnDirection::Response => "response",
            },
        ));
    }

    if let Some(payload) = event.payload.downcast_ref::<TurnAbortedPayload>() {
        attributes.push(KeyValue::new(
            "gen_ai.agent.abort_reason",
            payload.reason.clone(),
        ));
        if let Some(error_message) = &payload.error_message_safe {
            attributes.push(KeyValue::new(
                "gen_ai.agent.error_message",
                error_message.clone(),
            ));
        }
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

    if let Some(payload) = event.payload.downcast_ref::<ToolCallBeginPayload>() {
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
        attributes.push(KeyValue::new(
            "gen_ai.agent.name",
            payload.agent_code.clone(),
        ));
        attributes.push(KeyValue::new("gen_ai.tool.name", payload.tool_name.clone()));
        attributes.push(KeyValue::new("gen_ai.tool.status", payload.status.clone()));
        attributes.push(KeyValue::new(
            "gen_ai.tool.latency_ms",
            i64::try_from(payload.latency_ms).unwrap_or(i64::MAX),
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
        EventPayload, FinishReasonPayload, UnknownEventPayload, UsagePayload,
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
        let spans = InMemorySpanExporter::default();
        let resource_capture = Arc::new(Mutex::new(None));
        let provider = TracerProvider::builder()
            .with_simple_exporter(spans.clone())
            .with_simple_exporter(ResourceCapturingExporter {
                resource: resource_capture.clone(),
            })
            .with_resource(genai_resource("apxm-test"))
            .build();
        let exporter = GenAiEventExporter::new(&provider, config);
        (exporter, spans, resource_capture)
    }

    fn event(payload: impl EventPayload) -> ApxmEvent {
        ApxmEvent::root(payload, EventSource::Runtime, "trace-1").with_seq(7)
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
            },
            tool_calls: Vec::new(),
            response_id: None,
        }));
        exporter.emit(event(ToolStartPayload {
            name: "lookup".to_string(),
            args: std::collections::HashMap::new(),
        }));
        exporter.emit(event(TurnStartedPayload {
            execution_id: "exec-1".to_string(),
            turn_id: None,
            coordinator_label: None,
        }));
        exporter.emit(event(SubagentSpawnBeginPayload {
            agent_code: "worker".to_string(),
            agent_name: None,
            agent_type: None,
            module_key: None,
            autonomy_policy: None,
            parent_span_id: None,
        }));

        let spans = spans.get_finished_spans().expect("span export failed");
        let operations: Vec<_> = spans
            .iter()
            .filter_map(|span| attribute_value(span, "gen_ai.operation.name"))
            .map(Value::as_str)
            .map(|value| value.into_owned())
            .collect();
        assert_eq!(
            operations,
            vec![
                GENAI_OPERATION_INVOKE_WORKFLOW,
                GENAI_OPERATION_CHAT,
                GENAI_OPERATION_EXECUTE_TOOL,
                GENAI_OPERATION_INVOKE_AGENT,
                GENAI_OPERATION_CREATE_AGENT,
            ]
        );
    }

    #[test]
    fn llm_step_and_turn_boundary_export_typed_observability_attributes() {
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
        }));
        exporter.emit(event(TurnBoundaryPayload {
            turn_number: 3,
            direction: TurnDirection::Response,
        }));

        let spans = spans.get_finished_spans().expect("span export failed");
        assert_eq!(spans.len(), 2);
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
        assert_eq!(
            attribute_value(&spans[1], "gen_ai.agent.turn.number"),
            Some(&Value::I64(3))
        );
        assert_eq!(
            attribute_value(&spans[1], "gen_ai.agent.turn.direction")
                .map(Value::as_str)
                .map(|value| value.into_owned()),
            Some("response".to_string())
        );
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
            },
            tool_calls: vec![ToolCallPayload {
                id: "call-1".to_string(),
                name: "vault".to_string(),
                arguments: arguments.clone(),
            }],
            response_id: None,
        }));

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
            },
            tool_calls: vec![ToolCallPayload {
                id: "call-1".to_string(),
                name: "vault".to_string(),
                arguments: arguments.clone(),
            }],
            response_id: None,
        }));

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
