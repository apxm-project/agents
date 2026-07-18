//! LLM operations handler - Unified handler for Ask, Think, Reason
//!
//! Three operation types, one underlying LLM executor:
//! - Ask: Simple Q&A (LOW latency) - returns plain text
//! - Think: Extended thinking with budget (HIGH latency) - deep reasoning
//! - Reason: Structured reasoning (MEDIUM latency) - belief/goal updates
//!
//! The operation type serves as a marker for runtime config lookup.
//! Actual LLM parameters come from runtime configuration.
//!
//! ## Tool Support (V1)
//!
//! The Ask operation supports tool usage via a tool loop:
//! 1. Send prompt + tool schemas to LLM
//! 2. LLM returns tool_calls (or text)
//! 3. Execute tool calls via CapabilitySystem
//! 4. Feed results back to LLM
//! 5. Repeat until LLM returns text (no tool calls)

use super::{
    ExecutionContext, Node, Result, Value, apply_llm_request_routing_from_node,
    copy_llm_request_routing, execute_llm_request_for_node_with_context,
    get_optional_string_attribute, get_optional_u64_attribute, get_string_attribute,
    template::{
        PromptRequestDisposition, llm_input_bindings_from_node, prompt_request_disposition,
        render_named,
    },
    warmup::{dispatch_warmup, should_dispatch_warmup},
};
use crate::aam::TransitionLabel;
use crate::context_envelope::resolve_serialized as resolve_sealed_context;
use crate::context_stack::{
    ContextPermissionScope, ContextPlan, ContextPlanMetrics, ContextPlanningError, ContextScope,
    ContextSegmentRole, ContextSegmentSpec, ContextSensitivity,
};
use crate::executor::memoization::{MemoCache, MemoizationIdentity};
use apxm_backends::llm::backends::request::{Message, Role};
use apxm_backends::llm::backends::traits::ResponseMemoizationPolicy;
use apxm_backends::llm::backends::vllm::attrs as vllm_attrs;
use apxm_backends::{LLMRequest, LLMResponse, ToolChoice};
use apxm_capability_iface::events::{
    ModelContextCallKind, ModelContextMetrics, ModelContextPlanStatus,
};
use apxm_core::apxm_llm;
use apxm_core::constants::graph::attrs::PromptInputRole;
use apxm_core::constants::{
    extra_body as extra_body_keys,
    graph::{attrs as graph_attrs, metadata as graph_meta},
    runtime::belief_keys,
};
use apxm_core::error::RuntimeError;
use apxm_core::types::operations::AISOperationType;
use apxm_core::types::{ApxmGraphHints, PriorityClass};
use serde_json::Value as JsonValue;
use std::collections::BTreeMap;

pub(super) mod pipeline;
pub(super) mod structured_output;
pub(super) mod tool_dispatch;
pub(crate) use tool_dispatch::script_tool_policy;

use pipeline::{
    charge_tokens, effort_token_budget, resolve_global_token_budget,
    resolve_node_output_token_limit,
};
use structured_output::{
    build_schema_retry_prompt, output_schema_from_node, parse_structured_output,
    process_structured_output, validate_against_output_schema,
};
use tool_dispatch::{execute_ask_with_tools, resolve_ask_tools};
// Re-exported so other handlers (e.g. AUTONOMOUS) can run the same dynamic
// model->tool->model loop, letting an agent actually execute the tools it
// decides to use rather than only reasoning about them.
pub(crate) use tool_dispatch::{
    execute_ask_with_tools as run_tool_loop, resolve_ask_tools as resolve_node_tools,
};

/// Admit a non-node model request through the same request-shape and token
/// planning boundary used by provider dispatch.
pub(crate) fn admit_model_egress(
    ctx: &ExecutionContext,
    request: &LLMRequest,
) -> Result<pipeline::ModelCallAdmission> {
    request
        .validate_provider_dispatch()
        .map_err(|error| RuntimeError::LLM {
            message: format!("provider request admission failed: {error}"),
            backend: request.backend.clone(),
        })?;
    pipeline::admit_model_call(ctx, request)
}

/// LLM operation mode (derived from operation type)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmMode {
    /// Simple Q&A - no extended thinking, plain text response
    Ask,
    /// Extended thinking with token_budget
    Think,
    /// Structured reasoning with belief/goal updates
    Reason,
}

#[cfg(test)]
mod context_request_tests {
    use std::collections::BTreeMap;
    use std::collections::HashMap;
    use std::sync::Arc;

    use apxm_backends::LLMRegistry;
    use apxm_backends::llm::backends::MockLLMBackend;
    use apxm_backends::llm::backends::request::Role;
    use apxm_core::types::AISOperationType;

    use super::*;
    use crate::aam::Aam;
    use crate::capability::CapabilitySystem;
    use crate::context_stack::{ContextPlanningPolicy, ContextStack, ContextTokenizer, ScopeRules};
    use crate::memory::{MemoryConfig, MemorySystem};

    const TEST_PROFILE: &str = "contextual-request-test";
    const RECORDED_BACKEND: &str = "recorded-boundary";
    const RECORDED_MODEL: &str = "recorded-boundary-model";

    async fn test_execution_context() -> ExecutionContext {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .expect("memory"),
        );
        let aam = Aam::new();
        let capability_system = Arc::new(CapabilitySystem::with_aam(aam.clone()));
        ExecutionContext::new(memory, Arc::new(LLMRegistry::new()), capability_system, aam)
    }

    #[tokio::test]
    async fn contextualized_node_request_renders_context_and_reports_only_aggregates() {
        let mut ctx = test_execution_context().await;
        let context_dir = tempfile::tempdir().expect("temporary context directory");
        ctx.context_stack = Some(Arc::new(
            ContextStack::new(
                context_dir.path().to_path_buf(),
                Arc::new(HashMap::new()),
                Arc::new(Vec::new()),
            )
            .with_planning_policy(ContextPlanningPolicy {
                tokenizer: ContextTokenizer::O200kBase,
                token_budget: 1_024,
                profiles: BTreeMap::from([(
                    TEST_PROFILE.to_owned(),
                    ScopeRules {
                        upstream_depth: 1,
                        upstream_frame_budget: 256,
                        session_frame_budget: 128,
                        include_upstream_prompts: false,
                    },
                )]),
            }),
        ));
        let mut node = Node::new(17, AISOperationType::Ask);
        node.attributes.insert(
            graph_attrs::PROFILE.to_owned(),
            Value::String(TEST_PROFILE.to_owned()),
        );

        let contextual = contextualize_node_request(
            &ctx,
            &node,
            LLMRequest::new("answer").with_system_prompt("base policy"),
        )
        .expect("configured context profile");

        let system_prompt = contextual
            .request
            .system_prompt
            .as_deref()
            .expect("contextual request has a system prompt");
        assert!(system_prompt.contains("## Session"));
        assert!(system_prompt.contains("base policy"));
        assert_eq!(contextual.metrics.node_id, Some(node.id));
        assert_eq!(contextual.metrics.call_kind, ModelContextCallKind::Node);
        assert_eq!(
            contextual.metrics.plan_status,
            ModelContextPlanStatus::Assembled
        );
        assert!(contextual.metrics.token_budget.is_some());
        assert!(contextual.plan.is_some());
    }

    #[tokio::test]
    async fn contextualized_node_request_reports_unplanned_without_a_context_stack() {
        let ctx = test_execution_context().await;
        let node = Node::new(18, AISOperationType::Ask);

        let contextual = contextualize_node_request(
            &ctx,
            &node,
            LLMRequest::new("answer").with_system_prompt("base policy"),
        )
        .expect("no context stack remains unplanned");

        assert_eq!(
            contextual.request.system_prompt.as_deref(),
            Some("base policy")
        );
        assert_eq!(
            contextual.metrics.plan_status,
            ModelContextPlanStatus::Unplanned
        );
        assert!(contextual.metrics.token_budget.is_none());
        assert!(contextual.metrics.original_tokens.is_none());
        assert!(contextual.plan.is_none());
    }

    #[tokio::test]
    async fn malformed_sealed_context_transport_rejects_before_model_context_assembly() {
        let mut ctx = test_execution_context().await;
        ctx.metadata.insert(
            crate::metadata_keys::SEALED_CONTEXT_TRANSPORT_V1.to_string(),
            "not-json".to_string(),
        );

        let error = contextualize_node_request(
            &ctx,
            &Node::new(27, AISOperationType::Ask),
            LLMRequest::new("answer"),
        )
        .err()
        .expect("malformed sealed context must fail before model dispatch");
        assert!(
            error
                .to_string()
                .contains("sealed model context failed validation")
        );
    }

    #[test]
    fn context_metrics_without_a_plan_are_always_unplanned() {
        let metrics = model_context_metrics(
            Some(18),
            ModelContextCallKind::ToolContinuation,
            ModelContextPlanStatus::Inherited,
            None,
        );

        assert_eq!(metrics.plan_status, ModelContextPlanStatus::Unplanned);
        assert!(metrics.token_budget.is_none());
        assert!(metrics.admitted_tokens.is_none());
    }

    #[test]
    fn context_profile_requires_node_configuration() {
        let mut configured = Node::new(19, AISOperationType::Ask);
        configured.attributes.insert(
            graph_attrs::PROFILE.to_owned(),
            Value::String(TEST_PROFILE.to_owned()),
        );

        assert_eq!(context_profile_for_node(&configured), Ok(TEST_PROFILE));
        assert_eq!(
            context_profile_for_node(&Node::new(20, AISOperationType::Ask)),
            Err(ContextPlanningError::ProfileNotSpecified)
        );
    }

    #[test]
    fn memoization_requires_a_compiler_written_guard() {
        let mut stamped = Node::new(23, AISOperationType::Ask);
        stamped
            .attributes
            .insert(graph_attrs::MEMOIZABLE.to_string(), Value::Bool(true));
        let mut rejected = Node::new(24, AISOperationType::Ask);
        rejected
            .attributes
            .insert(graph_attrs::MEMOIZABLE.to_string(), Value::Bool(false));

        assert!(compiler_stamped_memoization(&stamped));
        assert!(!compiler_stamped_memoization(&rejected));
        assert!(!compiler_stamped_memoization(&Node::new(
            25,
            AISOperationType::Ask
        )));
    }

    #[test]
    fn memoization_request_identity_includes_generation_controls() {
        let base = LLMRequest::new("answer")
            .with_temperature(0.0)
            .with_max_tokens(64);
        let different_limit = LLMRequest::new("answer")
            .with_temperature(0.0)
            .with_max_tokens(128);

        assert_ne!(
            memoization_request_identity(&base),
            memoization_request_identity(&different_limit)
        );
        assert!(memoization_request_identity(&LLMRequest::new("answer")).is_none());
    }

    #[tokio::test]
    async fn recorded_backend_routes_user_and_system_roles_without_session_path() {
        let mut ctx = test_execution_context().await;
        let context_dir = tempfile::tempdir().expect("temporary context directory");
        let context_path = context_dir.path().to_string_lossy().into_owned();
        ctx.context_stack = Some(Arc::new(
            ContextStack::new(
                context_dir.path().to_path_buf(),
                Arc::new(HashMap::new()),
                Arc::new(Vec::new()),
            )
            .with_planning_policy(ContextPlanningPolicy {
                tokenizer: ContextTokenizer::O200kBase,
                token_budget: 1_024,
                profiles: BTreeMap::from([(
                    TEST_PROFILE.to_owned(),
                    ScopeRules {
                        upstream_depth: 0,
                        upstream_frame_budget: 0,
                        session_frame_budget: 128,
                        include_upstream_prompts: false,
                    },
                )]),
            }),
        ));
        let mock = MockLLMBackend::static_response("ok").model_name(RECORDED_MODEL);
        ctx.llm_registry
            .register(RECORDED_BACKEND, mock.clone())
            .expect("register recorded backend");
        ctx.llm_registry
            .set_model_route(RECORDED_MODEL, RECORDED_BACKEND)
            .expect("register recorded model route");

        let mut node = Node::new(21, AISOperationType::Ask);
        node.attributes.extend([
            (
                graph_attrs::TEMPLATE_STR.to_string(),
                Value::String("Answer: {question}".to_string()),
            ),
            (
                graph_attrs::INPUT_NAMES.to_string(),
                Value::Array(vec![
                    Value::String("question".to_string()),
                    Value::String("policy".to_string()),
                    Value::String("upstream_value".to_string()),
                ]),
            ),
            (
                graph_attrs::INPUT_ROLES.to_string(),
                Value::Array(vec![
                    Value::String("user".to_string()),
                    Value::String("system".to_string()),
                    Value::String("dependency_only".to_string()),
                ]),
            ),
            (
                graph_attrs::PROFILE.to_string(),
                Value::String(TEST_PROFILE.to_owned()),
            ),
            (
                graph_attrs::BACKEND.to_string(),
                Value::String(RECORDED_BACKEND.to_string()),
            ),
            (
                graph_attrs::MODEL.to_string(),
                Value::String(RECORDED_MODEL.to_string()),
            ),
            (graph_attrs::TOKEN_BUDGET.to_string(), Value::from(128_i64)),
        ]);

        execute(
            &ctx,
            &node,
            vec![
                Value::String("What is the status?".to_string()),
                Value::String("Follow policy P.".to_string()),
                Value::String("dependency secret".to_string()),
            ],
        )
        .await
        .expect("typed roles dispatch through the recorded backend");

        let calls = mock.recorded_calls();
        assert_eq!(calls.len(), 1);
        let call = &calls[0];
        assert_eq!(
            call.messages
                .iter()
                .map(|message| message.role.clone())
                .collect::<Vec<_>>(),
            vec![Role::System, Role::User]
        );
        assert_eq!(
            call.messages[1].text_content(),
            "Answer: What is the status?"
        );
        let provider_text = call
            .messages
            .iter()
            .map(|message| message.text_content())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(provider_text.contains("Follow policy P."));
        assert!(!provider_text.contains("dependency secret"));
        assert!(!provider_text.contains(&context_path));
    }

    #[tokio::test]
    async fn uncorrelated_tool_context_fails_before_the_recorded_backend_is_called() {
        let ctx = test_execution_context().await;
        let mock = MockLLMBackend::static_response("must not run");
        ctx.llm_registry
            .register("tool-context-boundary", mock.clone())
            .expect("register tool-context backend");

        let mut node = Node::new(22, AISOperationType::Ask);
        node.attributes.extend([
            (
                graph_attrs::TEMPLATE_STR.to_string(),
                Value::String("No provider tool payload".to_string()),
            ),
            (
                graph_attrs::INPUT_NAMES.to_string(),
                Value::Array(vec![Value::String("tool_result".to_string())]),
            ),
            (
                graph_attrs::INPUT_ROLES.to_string(),
                Value::Array(vec![Value::String("tool_context".to_string())]),
            ),
            (
                graph_attrs::BACKEND.to_string(),
                Value::String("tool-context-boundary".to_string()),
            ),
        ]);

        let error = execute(
            &ctx,
            &node,
            vec![Value::String("untrusted tool context".to_string())],
        )
        .await
        .expect_err("uncorrelated tool context cannot become provider payload");

        assert!(matches!(
            error,
            RuntimeError::Executor(message)
                if message.contains("without a correlated provider tool call")
        ));
        assert_eq!(mock.call_count(), 0);
    }

    #[tokio::test]
    async fn control_role_fails_before_the_recorded_backend_is_called() {
        let ctx = test_execution_context().await;
        let mock = MockLLMBackend::static_response("must not run");
        ctx.llm_registry
            .register("control-boundary", mock.clone())
            .expect("register control backend");

        let mut node = Node::new(22, AISOperationType::Ask);
        node.attributes.extend([
            (
                graph_attrs::TEMPLATE_STR.to_string(),
                Value::String("No provider control payload".to_string()),
            ),
            (
                graph_attrs::INPUT_NAMES.to_string(),
                Value::Array(vec![Value::String("authorization".to_string())]),
            ),
            (
                graph_attrs::INPUT_ROLES.to_string(),
                Value::Array(vec![Value::String("control".to_string())]),
            ),
            (
                graph_attrs::BACKEND.to_string(),
                Value::String("control-boundary".to_string()),
            ),
        ]);

        let error = execute(
            &ctx,
            &node,
            vec![Value::String("capability grant".to_string())],
        )
        .await
        .expect_err("control inputs cannot become provider payload");

        assert!(matches!(
            error,
            RuntimeError::Executor(message)
                if message.contains("cannot cross the provider request boundary")
        ));
        assert_eq!(mock.call_count(), 0);
    }
}

impl LlmMode {
    fn name(self) -> &'static str {
        match self {
            LlmMode::Ask => "ASK",
            LlmMode::Think => "THINK",
            LlmMode::Reason => "REASON",
        }
    }
}

impl From<&AISOperationType> for LlmMode {
    fn from(op_type: &AISOperationType) -> Self {
        match op_type {
            AISOperationType::Ask => LlmMode::Ask,
            AISOperationType::Think => LlmMode::Think,
            AISOperationType::Reason => LlmMode::Reason,
            _ => LlmMode::Ask,
        }
    }
}

struct ProviderBoundInputs {
    user_inputs: Vec<Value>,
    user_input_names: Vec<String>,
    system_prompt: Option<String>,
    tool_messages: Vec<Message>,
    excluded_inputs: Vec<(String, Value)>,
}

fn provider_input_text(value: &Value) -> String {
    value
        .as_string()
        .map(|text| text.to_owned())
        .unwrap_or_else(|| value.to_string())
}

/// Route every typed input through its explicit provider-bound disposition.
fn provider_bound_inputs(
    bindings: &[(String, PromptInputRole)],
    inputs: &[Value],
) -> Result<ProviderBoundInputs> {
    let mut user_inputs = Vec::new();
    let mut user_input_names = Vec::new();
    let mut system_inputs = Vec::new();
    let tool_messages = Vec::new();
    let mut excluded_inputs = Vec::new();

    for (index, (name, role)) in bindings.iter().enumerate() {
        let value = inputs.get(index).ok_or_else(|| {
            RuntimeError::Executor(format!(
                "input '{name}' at position {index} is absent from the LLM request"
            ))
        })?;
        match prompt_request_disposition(name, *role)? {
            PromptRequestDisposition::UserTemplate => {
                user_inputs.push(value.clone());
                user_input_names.push(name.clone());
            }
            PromptRequestDisposition::SystemPrompt => {
                system_inputs.push(provider_input_text(value));
            }
            PromptRequestDisposition::Excluded => {
                excluded_inputs.push((name.clone(), value.clone()));
            }
        }
    }

    Ok(ProviderBoundInputs {
        user_inputs,
        user_input_names,
        system_prompt: (!system_inputs.is_empty()).then(|| system_inputs.join("\n\n")),
        tool_messages,
        excluded_inputs,
    })
}

fn resolve_system_prompt(
    ctx: &ExecutionContext,
    node: &Node,
    mode: LlmMode,
    dataflow_prompt: Option<String>,
) -> Result<String> {
    let (config_instruction, template_name, fallback) = match mode {
        LlmMode::Ask => (
            ctx.instruction_config.ask.as_ref(),
            "ask_system",
            "You are a helpful AI assistant. Answer concisely.",
        ),
        LlmMode::Think => (
            ctx.instruction_config.think.as_ref(),
            "think_system",
            "You are a deep reasoning AI. Think through problems carefully and thoroughly.",
        ),
        LlmMode::Reason => (
            ctx.instruction_config.reason.as_ref(),
            "reason_system",
            "You are a helpful AI assistant. When providing structured responses, \
 use JSON format with fields: belief_updates (object), new_goals (array), \
 and result (any type).",
        ),
    };
    // A dataflow operand bound to `__system` takes precedence over the static
    // attribute so authors can inject context per turn (constitution #5).
    let prompt = dataflow_prompt
        .filter(|s| !s.is_empty())
        .or_else(|| {
            get_optional_string_attribute(node, graph_attrs::SYSTEM_PROMPT)
                .ok()
                .flatten()
        })
        .or_else(|| config_instruction.cloned())
        .or_else(|| apxm_backends::render_prompt(template_name, &serde_json::json!({})).ok())
        .unwrap_or_else(|| fallback.to_string());
    Ok(prompt)
}

struct ContextSystemPrompt {
    system_prompt: String,
    plan: Option<ContextPlan>,
}

/// Resolve the host-owned sealed transport without interpreting any catalogue
/// or activation metadata.
fn sealed_context_messages(ctx: &ExecutionContext) -> Result<Vec<Message>> {
    let Some(serialized) = ctx
        .metadata
        .get(crate::metadata_keys::SEALED_CONTEXT_TRANSPORT_V1)
    else {
        return Ok(Vec::new());
    };
    resolve_sealed_context(serialized)
        .map(|context| context.messages)
        .map_err(|error| RuntimeError::LLM {
            message: format!("sealed model context failed validation: {error}"),
            backend: None,
        })
}

/// A model request with the execution-owned context boundary applied.
pub(crate) struct ContextualNodeRequest {
    /// Request with the admitted context rendered into its system prompt.
    pub(crate) request: LLMRequest,
    /// Aggregate-only evidence for the first request in this node call.
    pub(crate) metrics: ModelContextMetrics,
    /// Typed plan a tool continuation extends without assembling again.
    pub(crate) plan: Option<ContextPlan>,
}

fn compose_context_stack_system_prompt(
    ctx: &ExecutionContext,
    node: &Node,
    system_prompt: String,
) -> std::result::Result<ContextSystemPrompt, ContextPlanningError> {
    let Some(stack) = &ctx.context_stack else {
        return Ok(ContextSystemPrompt {
            system_prompt,
            plan: None,
        });
    };
    let profile = context_profile_for_node(node)?;
    let assembly = stack.assemble(node.id, profile)?;
    if assembly.frames.is_empty() {
        return Ok(ContextSystemPrompt {
            system_prompt,
            plan: Some(assembly.plan),
        });
    }
    Ok(ContextSystemPrompt {
        system_prompt: format!("{}\n\n---\n\n{}", system_prompt, assembly),
        plan: Some(assembly.plan),
    })
}

fn request_segment_spec(
    role: ContextSegmentRole,
    provenance: impl Into<String>,
    protected: bool,
    max_tokens: usize,
) -> ContextSegmentSpec {
    ContextSegmentSpec {
        scope: ContextScope::Local,
        role,
        provenance: provenance.into(),
        permission: ContextPermissionScope::Local,
        sensitivity: ContextSensitivity::Private,
        protected,
        max_tokens,
    }
}

fn record_request_message(
    plan: &mut ContextPlan,
    message: &Message,
    provenance: String,
) -> std::result::Result<(), ContextPlanningError> {
    let role = match message.role {
        Role::System => ContextSegmentRole::System,
        Role::User | Role::Assistant => ContextSegmentRole::User,
        Role::Tool => ContextSegmentRole::Tool,
    };
    let max_tokens = plan.remaining_tokens();
    plan.admit(
        request_segment_spec(role, provenance, true, max_tokens),
        &message.text_content(),
    )?;
    Ok(())
}

/// Resolve the context profile declared by the graph node.
///
/// Context planning cannot infer a profile because its scope rules and token
/// budget are capability evidence rather than runtime defaults.
pub(crate) fn context_profile_for_node(
    node: &Node,
) -> std::result::Result<&str, ContextPlanningError> {
    node.attributes
        .get(graph_attrs::PROFILE)
        .and_then(|value| value.as_str())
        .filter(|profile| !profile.trim().is_empty())
        .ok_or(ContextPlanningError::ProfileNotSpecified)
}

/// Preserve the typed planning failure until the operation boundary converts it
/// into the runtime's public error surface.
pub(crate) fn context_planning_runtime_error(
    node: &Node,
    error: ContextPlanningError,
) -> RuntimeError {
    RuntimeError::Operation {
        op_type: node.op_type,
        message: format!("context planning failed: {error}"),
    }
}

/// Apply the shared context plan to any model request issued for a graph node.
///
/// The rendered context stays inside the provider request. Only its aggregate
/// packing evidence reaches observability, while a tool continuation extends
/// the returned plan rather than assembling a second context path.
pub(crate) fn contextualize_node_request(
    ctx: &ExecutionContext,
    node: &Node,
    request: LLMRequest,
) -> Result<ContextualNodeRequest> {
    let original_messages = request.resolved_messages();
    let sealed_messages = sealed_context_messages(ctx)?;
    let system_prompt = request.system_prompt.clone().unwrap_or_default();
    let contextual = compose_context_stack_system_prompt(ctx, node, system_prompt)
        .map_err(|error| context_planning_runtime_error(node, error))?;
    let mut request = request.with_system_prompt(contextual.system_prompt);
    let messages_for_plan = if sealed_messages.is_empty() {
        original_messages
    } else {
        let mut messages = request.resolved_messages();
        let insertion_index = messages
            .iter()
            .take_while(|message| message.role == Role::System)
            .count();
        messages.splice(insertion_index..insertion_index, sealed_messages);
        request.system_prompt = None;
        request.messages = messages.clone();
        messages
    };
    let mut plan = contextual.plan.or_else(|| {
        ctx.context_planning
            .as_ref()
            .and_then(|policy| ContextPlan::from_policy(policy, None).ok())
    });
    if let Some(plan) = plan.as_mut() {
        for (index, message) in messages_for_plan.iter().enumerate() {
            record_request_message(plan, message, format!("request:message:{index}"))
                .map_err(|error| context_planning_runtime_error(node, error))?;
        }
    }
    let plan_metrics = plan.as_ref().map(ContextPlan::metrics);
    let plan_status = if plan_metrics.is_some() {
        ModelContextPlanStatus::Assembled
    } else {
        ModelContextPlanStatus::Unplanned
    };
    Ok(ContextualNodeRequest {
        request,
        metrics: model_context_metrics(
            Some(node.id),
            ModelContextCallKind::Node,
            plan_status,
            plan_metrics.as_ref(),
        ),
        plan,
    })
}

/// Dispatch a node-owned model request through the shared context boundary.
pub(crate) async fn execute_contextual_node_request(
    ctx: &ExecutionContext,
    node: &Node,
    phase: &str,
    request: &LLMRequest,
) -> Result<LLMResponse> {
    let contextual = contextualize_node_request(ctx, node, request.clone())?;
    execute_llm_request_for_node_with_context(
        ctx,
        node,
        phase,
        &contextual.request,
        &contextual.metrics,
    )
    .await
}

/// Construct context-plan aggregates without exposing content-bearing fields.
pub(crate) fn model_context_metrics(
    node_id: Option<u64>,
    call_kind: ModelContextCallKind,
    plan_status: ModelContextPlanStatus,
    plan_metrics: Option<&ContextPlanMetrics>,
) -> ModelContextMetrics {
    let plan_status = if plan_metrics.is_some() {
        plan_status
    } else {
        ModelContextPlanStatus::Unplanned
    };
    match plan_metrics {
        Some(plan) => ModelContextMetrics {
            node_id,
            call_kind,
            plan_status,
            token_budget: Some(plan.token_budget),
            original_tokens: Some(plan.original_tokens),
            admitted_tokens: Some(plan.admitted_tokens),
            kept_segments: Some(plan.kept_segments),
            truncated_segments: Some(plan.truncated_segments),
            omitted_token_budget_segments: Some(plan.omitted_token_budget_segments),
            omitted_empty_segments: Some(plan.omitted_empty_segments),
            generation: None,
        },
        None => ModelContextMetrics {
            node_id,
            call_kind,
            plan_status,
            token_budget: None,
            original_tokens: None,
            admitted_tokens: None,
            kept_segments: None,
            truncated_segments: None,
            omitted_token_budget_segments: None,
            omitted_empty_segments: None,
            generation: None,
        },
    }
}

/// Emit context-plan aggregates for model calls that do not use the shared
/// node-dispatch helper.
pub(crate) fn emit_model_context_metrics(
    ctx: &ExecutionContext,
    node_id: Option<u64>,
    call_kind: ModelContextCallKind,
    plan_status: ModelContextPlanStatus,
    plan_metrics: Option<&ContextPlanMetrics>,
) {
    if let Some(emitter) = &ctx.event_emitter {
        emitter.emit_model_context_metrics(&model_context_metrics(
            node_id,
            call_kind,
            plan_status,
            plan_metrics,
        ));
    }
}

/// Build APXM graph hints from a node's graph attributes and attach them to
/// `request`. Hints are then enriched with runtime-only identifiers
/// (execution id, human-readable node name, runtime-derived priority class
/// fallback) that the compiler cannot supply.
pub(crate) fn attach_graph_hints(
    ctx: &ExecutionContext,
    node: &Node,
    request: LLMRequest,
) -> LLMRequest {
    if let Ok(node_id) = u32::try_from(node.id)
        && let Some(hints) = ctx.dispatch_hints_for_node(node_id)
    {
        apxm_llm!(debug,
         execution_id = %ctx.execution_id,
         graph_id = %ctx.graph_id,
         node_id = node.id,
         priority_class = ?hints.priority_class,
         reuse_group = ?hints.reuse_group,
         downstream = hints.downstream_nodes.len(),
         "Built APXM graph hints from DispatchIrV1"
        );
        return request.with_apxm_hints(hints);
    }

    let node_name = node
        .metadata
        .name
        .clone()
        .unwrap_or_else(|| format!("{}{}", graph_meta::GENERATED_NODE_NAME_PREFIX, node.id));

    let mut hints =
        ApxmGraphHints::from_node_attrs(ctx.graph_id.clone(), node_name.clone(), &node.attributes);

    // Enrich with runtime-only fields the compiler cannot stamp.
    hints.execution_id = Some(ctx.execution_id.clone());
    hints.node_id = Some(node.id as u32);
    hints.node_name = Some(node_name);

    // If the compiler did not stamp a class, derive one from the node priority.
    if hints.priority_class.is_none() {
        let priority_value = node.metadata.priority;
        hints.priority_class = Some(
            if i64::from(priority_value) >= graph_meta::CRITICAL_PATH_PRIORITY_THRESHOLD {
                PriorityClass::CriticalPath
            } else {
                PriorityClass::Parallel
            },
        );
    }

    apxm_llm!(debug,
     execution_id = %ctx.execution_id,
     graph_id = %ctx.graph_id,
     node_id = node.id,
     priority_class = ?hints.priority_class,
     reuse_group = ?hints.reuse_group,
     downstream = hints.downstream_nodes.len(),
     "Built APXM graph hints for backend scheduling"
    );

    request.with_apxm_hints(hints)
}

/// Resolve a salt selector sentinel to its concrete string. Literal
/// selectors pass through unchanged.
fn substitute_cache_salt_selector(ctx: &ExecutionContext, selector: &str) -> String {
    match selector {
        vllm_attrs::CACHE_SALT_EXECUTION | vllm_attrs::CACHE_SALT_EXECUTION_ID => {
            ctx.execution_id.clone()
        }
        vllm_attrs::CACHE_SALT_GRAPH_EXECUTION => {
            format!("{}:{}", ctx.graph_id, ctx.execution_id)
        }
        literal => literal.to_string(),
    }
}

/// Attach a `cache_salt` entry to `request.extra_body`, preserving any
/// existing object fields. No-ops if `cache_salt` is empty.
fn attach_cache_salt(mut request: LLMRequest, cache_salt: String) -> LLMRequest {
    if cache_salt.is_empty() {
        return request;
    }
    let mut extra = request
        .extra_body
        .take()
        .unwrap_or_else(|| JsonValue::Object(Default::default()));
    if !extra.is_object() {
        extra = JsonValue::Object(Default::default());
    }
    if let JsonValue::Object(ref mut map) = extra {
        map.insert(
            extra_body_keys::CACHE_SALT_KEY.to_string(),
            JsonValue::String(cache_salt),
        );
    }
    request.extra_body = Some(extra);
    request
}

/// Resolution chain for the vLLM `cache_salt`:
///
/// 1. Explicit `vllm_cache_salt` node attribute — author intent always wins.
///    A value of `"none"` (or empty) disables salting entirely.
/// 2. Compiler-stamped `shared_prefix_group` — the SharedPrefixAnalysis
///    pass marks sibling nodes that share a bit-identical leading prompt.
///    Salting by `{graph_id}:{group}` lets the vLLM prefix cache survive
///    across executions of the same graph for grouped nodes, while the
///    benchmark harness can still keep ungrouped nodes execution-isolated
///    via `APXM_VLLM_CACHE_SALT=execution`.
/// 3. Env var fallback (`APXM_VLLM_CACHE_SALT`) — harness iteration
///    isolation for ungrouped nodes.
fn apply_vllm_request_overrides_from_node(
    ctx: &ExecutionContext,
    node: &Node,
    request: LLMRequest,
) -> Result<LLMRequest> {
    let explicit_attr = get_optional_string_attribute(node, vllm_attrs::CACHE_SALT_ATTR)?;
    if let Some(attr_value) = explicit_attr.as_deref() {
        match vllm_attrs::resolved_cache_salt_selector(Some(attr_value)) {
            Some(selector) => {
                let cache_salt = substitute_cache_salt_selector(ctx, &selector);
                return Ok(attach_cache_salt(request, cache_salt));
            }
            // Explicit "none"/empty: caller asked for no salting; do not
            // fall through to the compiler hint or env var.
            None => return Ok(request),
        }
    }

    let reuse_group = get_optional_string_attribute(node, graph_attrs::REUSE_GROUP)?
        .map(|g| g.trim().to_owned())
        .filter(|g| !g.is_empty());
    if let Some(group) = reuse_group {
        let cache_salt = format!("{}:{}", ctx.graph_id, group);
        return Ok(attach_cache_salt(request, cache_salt));
    }

    let Some(selector) = vllm_attrs::resolved_cache_salt_selector(None) else {
        return Ok(request);
    };
    let cache_salt = substitute_cache_salt_selector(ctx, &selector);
    Ok(attach_cache_salt(request, cache_salt))
}

/// Execute LLM operation - unified handler for Ask, Think, Reason
///
/// # Mode Behavior
///
/// - **Ask**: Simple Q&A, returns plain text, no structured parsing
/// - **Think**: Extended thinking with token_budget, uses thinking mode
/// - **Reason**: Structured output with belief_updates, new_goals, inner_plan
pub async fn execute(ctx: &ExecutionContext, node: &Node, inputs: Vec<Value>) -> Result<Value> {
    let mode = LlmMode::from(&node.op_type);
    let mode_name = mode.name();

    let base_prompt = get_string_attribute(node, graph_attrs::TEMPLATE_STR)
        .or_else(|_| get_string_attribute(node, graph_attrs::PROMPT))?;
    let budget = get_optional_u64_attribute(node, graph_attrs::BUDGET)?;
    let max_retries = node
        .attributes
        .get(graph_attrs::MAX_RETRIES)
        .and_then(|v| v.as_u64())
        .unwrap_or(3) as u32;
    let max_schema_retries = node
        .attributes
        .get(graph_attrs::MAX_SCHEMA_RETRIES)
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;
    let output_schema = output_schema_from_node(node)?;

    // Inner plan support only for Reason mode
    let supports_inner_plan = mode == LlmMode::Reason
        && node
            .attributes
            .get(graph_attrs::INNER_PLAN_SUPPORTED)
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
    let enable_inner_plan = mode == LlmMode::Reason
        && node
            .attributes
            .get(graph_attrs::ENABLE_INNER_PLAN)
            .and_then(|v| v.as_bool())
            .unwrap_or(supports_inner_plan);
    let bind_outputs = node
        .attributes
        .get(graph_attrs::BIND_INNER_PLAN_OUTPUTS)
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let bindings = llm_input_bindings_from_node(node, inputs.len())?;
    let provider_inputs = provider_bound_inputs(&bindings, &inputs)?;
    let prompt = render_named(
        &base_prompt,
        &provider_inputs.user_inputs,
        &provider_inputs.user_input_names,
    )?;

    let mut request = apply_llm_request_routing_from_node(
        LLMRequest::new(prompt.clone()).with_operation_type(node.op_type),
        node,
    )?;

    // Extended thinking. An explicit `effort` attr (off/low/medium/high) applies
    // to any LLM op and maps to a thinking token budget; without it, the Think op
    // still honors its `budget` attr as before. Backends that support extended
    // thinking (anthropic, vllm, ollama) lower this into their own request.
    let thinking_budget =
        effort_token_budget(node)?.or(if mode == LlmMode::Think { budget } else { None });
    if let Some(budget_tokens) = thinking_budget {
        request = request
            .with_thinking_token_budget(budget_tokens)
            .with_enable_thinking(true);
    }

    if let Some(max_tokens) = resolve_node_output_token_limit(node)? {
        request = request.with_max_tokens(max_tokens);
    }

    if let Some(schema) = output_schema.as_ref() {
        request = request.with_output_schema(schema.clone());
    }

    let system_prompt = resolve_system_prompt(ctx, node, mode, provider_inputs.system_prompt)?;
    let ContextualNodeRequest {
        mut request,
        metrics: _,
        plan: mut context_plan,
    } = contextualize_node_request(ctx, node, request.with_system_prompt(system_prompt))?;
    if !provider_inputs.tool_messages.is_empty() {
        if let Some(plan) = context_plan.as_mut() {
            for (index, message) in provider_inputs.tool_messages.iter().enumerate() {
                record_request_message(plan, message, format!("request:tool:{index}"))
                    .map_err(|error| context_planning_runtime_error(node, error))?;
            }
        }
        let mut messages = request.resolved_messages();
        messages.extend(provider_inputs.tool_messages);
        request = request.with_messages(messages);
    }
    if let Some(plan) = context_plan.as_mut() {
        for (name, value) in provider_inputs.excluded_inputs {
            plan.exclude(
                ContextSegmentSpec {
                    scope: ContextScope::Upstream(node.id),
                    role: ContextSegmentRole::Dependency,
                    provenance: format!("request:dependency:{name}"),
                    permission: ContextPermissionScope::GraphDependency,
                    sensitivity: ContextSensitivity::Private,
                    protected: true,
                    max_tokens: 0,
                },
                &provider_input_text(&value),
            );
        }
    }
    let context_plan_metrics = context_plan.as_ref().map(ContextPlan::metrics);
    // Tool configuration (Ask mode only). Tools are OPT-IN: a node only
    // attaches tools when it explicitly opts in via TOOLS (named list) or
    // TOOLS_ENABLED=true (use all registered capabilities). The default is
    // no tools — matches OpenAI/LangChain/CrewAI/PydanticAI behavior.
    if mode == LlmMode::Ask {
        let tools = resolve_ask_tools(ctx, node);

        if !tools.is_empty() {
            // If the resolved backend can't accept tool_choice="auto", proceed
            // text-only rather than hard-failing the turn: a conversational
            // agent should still answer when a backend lacks auto tool choice,
            // just without tools this turn. We warn once so the degradation is
            // visible. Configure `auto_tool_choice = false` in
            // `~/.apxm/config.toml` for servers that reject the field.
            let backend_name = ctx
                .llm_registry
                .resolve_backend_name(&request)
                .map_err(|e| RuntimeError::LLM {
                    message: format!("Failed to resolve backend for tool routing: {}", e),
                    backend: None,
                })?;
            let backend =
                ctx.llm_registry
                    .get_backend(&backend_name)
                    .ok_or_else(|| RuntimeError::LLM {
                        message: format!(
                            "Backend '{}' resolved but not present in registry",
                            backend_name
                        ),
                        backend: Some(backend_name.clone()),
                    })?;
            if backend.supports_auto_tool_choice() {
                apxm_llm!(debug,
                 execution_id = %ctx.execution_id,
                 tool_count = tools.len(),
                 "Attaching tools to ASK request"
                );
                request = request.with_tools(tools).with_tool_choice(ToolChoice::Auto);
            } else {
                apxm_llm!(warn,
                 execution_id = %ctx.execution_id,
                 backend = %backend_name,
                 tool_count = tools.len(),
                 "Backend does not support tool_choice=\"auto\"; proceeding \
                 text-only for this ASK (tools dropped this turn)"
                );
            }
        }
    }

    request = apply_vllm_request_overrides_from_node(ctx, node, request)?;

    // Attach APXM graph hints for graph-aware backends.
    request = attach_graph_hints(ctx, node, request);

    if let Some(estimated_prefix_tokens) = should_dispatch_warmup(ctx, node, &request) {
        dispatch_warmup(
            ctx,
            node.id,
            mode_name,
            &request,
            estimated_prefix_tokens,
            context_plan_metrics.as_ref(),
        )
        .await?;
    }

    // Execute with retries
    let mut last_error = None;
    let mut schema_retries_used = 0u32;
    for attempt in 0..=max_retries {
        // Check cancellation before each LLM attempt
        if ctx.cancellation_token.is_cancelled() {
            return Err(RuntimeError::SchedulerCancelled);
        }

        if attempt > 0 {
            apxm_llm!(warn,
             execution_id = %ctx.execution_id,
             mode = mode_name,
             attempt = attempt,
             "Retrying LLM operation"
            );

            // Exponential backoff
            let backoff_ms = 100 * 2_u64.pow(attempt - 1);
            tokio::time::sleep(tokio::time::Duration::from_millis(backoff_ms)).await;
        }

        match execute_llm_once(
            ctx,
            node,
            &request,
            mode,
            enable_inner_plan,
            bind_outputs,
            context_plan.as_ref(),
        )
        .await
        {
            Ok(value) => {
                if mode == LlmMode::Ask
                    && let Some(schema) = output_schema.as_ref()
                    && let Value::String(content) = &value
                    && let Err(validation_error) = validate_against_output_schema(content, schema)
                {
                    let validation_error_text = validation_error.to_string();
                    if schema_retries_used < max_schema_retries {
                        schema_retries_used = schema_retries_used.saturating_add(1);
                        request.prompt = build_schema_retry_prompt(
                            &request.prompt,
                            content,
                            schema,
                            &validation_error_text,
                        );
                        apxm_llm!(
                         warn,
                         execution_id = %ctx.execution_id,
                         retry = schema_retries_used,
                         max_schema_retries = max_schema_retries,
                         error = %validation_error_text,
                         "Retrying ASK due to output_schema validation failure"
                        );
                        continue;
                    }
                    return Err(validation_error);
                }
                return Ok(value);
            }
            Err(e) => {
                last_error = Some(e);
                apxm_llm!(debug,
                 execution_id = %ctx.execution_id,
                 mode = mode_name,
                 attempt = attempt,
                 error = %last_error.as_ref().unwrap(),
                 "LLM attempt failed"
                );
            }
        }
    }

    Err(last_error.unwrap_or_else(|| RuntimeError::LLM {
        message: format!("{} operation: all retry attempts exhausted", mode_name),
        backend: None,
    }))
}

/// Execute a single LLM attempt
async fn execute_llm_once(
    ctx: &ExecutionContext,
    node: &Node,
    request: &LLMRequest,
    mode: LlmMode,
    enable_inner_plan: bool,
    bind_outputs: bool,
    context_plan: Option<&ContextPlan>,
) -> Result<Value> {
    let mode_name = mode.name();
    let resolved_request =
        super::resolve_model_profile(ctx, request.clone()).map_err(|error| RuntimeError::LLM {
            message: error.to_string(),
            backend: request.backend.clone(),
        })?;
    let request = &resolved_request;

    request
        .validate_provider_dispatch()
        .map_err(|error| RuntimeError::LLM {
            message: format!("provider request admission failed: {error}"),
            backend: request.backend.clone(),
        })?;

    // For Ask mode with tools, use the tool loop
    if mode == LlmMode::Ask && request.has_tools() {
        return execute_ask_with_tools(ctx, node, request, context_plan).await;
    }

    let memoization_policy = ctx
        .llm_registry
        .resolve_backend_name(request)
        .ok()
        .and_then(|backend_name| ctx.llm_registry.get_backend(&backend_name))
        .map(|backend| backend.response_memoization_policy());
    let memoizable = compiler_stamped_memoization(node)
        && memoization_policy == Some(ResponseMemoizationPolicy::RuntimeExact);

    // Check memoization cache for deterministic (temperature=0) calls
    let memo_key = if memoizable {
        memoization_identity(ctx, request)
            .and_then(|identity| MemoCache::compute_key_with_identity(&identity))
    } else {
        None
    };
    if let Some(key) = memo_key
        && let Some(cached) = ctx.response_cache.get(key)
    {
        apxm_llm!(debug,
         execution_id = %ctx.execution_id,
         mode = mode_name,
         "Memoization cache hit"
        );
        if let Some(emitter) = &ctx.event_emitter {
            emitter.emit_memoization_hit(node.id);
        }
        charge_tokens(
            ctx,
            resolve_global_token_budget(ctx),
            cached.input_tokens + cached.output_tokens,
        )?;
        return match mode {
            LlmMode::Ask | LlmMode::Think => Ok(Value::String(cached.content)),
            LlmMode::Reason => {
                if let Ok(structured) = parse_structured_output(&cached.content) {
                    process_structured_output(
                        ctx,
                        node,
                        structured,
                        enable_inner_plan,
                        bind_outputs,
                    )
                    .await
                } else {
                    Ok(Value::String(cached.content))
                }
            }
        };
    }

    apxm_llm!(debug,
     execution_id = %ctx.execution_id,
     mode = mode_name,
     prompt_len = request.prompt.len(),
     "Sending LLM request"
    );

    // Execute LLM request through registry.
    let llm_start = std::time::Instant::now();
    // Resolve backend name BEFORE the call so we can attribute the
    // per-request honor evidence even if the call's intermediate
    // routing transforms the request shape. Failure here is non-fatal —
    // we want the LLM call to proceed even if backend-name attribution
    // is unavailable, since the metadata-bearing response still
    // surfaces token + timing evidence.
    let pre_call_backend = ctx.llm_registry.resolve_backend_name(&request).ok();
    let context_plan_metrics = context_plan.map(ContextPlan::metrics);
    let context_metrics = model_context_metrics(
        Some(node.id),
        ModelContextCallKind::Node,
        ModelContextPlanStatus::Assembled,
        context_plan_metrics.as_ref(),
    );
    let response =
        execute_llm_request_for_node_with_context(ctx, node, mode_name, request, &context_metrics)
            .await?;
    let total_ms = llm_start.elapsed().as_secs_f64() * 1000.0;

    // Fold per-request `x-apxm-fields-honored`
    // evidence (parsed by the OpenAI backend into
    // `response.metadata["fields_honored"]`) into the per-execution
    // collector. The collector union'd snapshot lands in
    // `dispatch_ir_metrics.fields_honored` at execution end.
    if let Some(backend_name) = pre_call_backend.as_deref()
        && let Some(serde_json::Value::Array(fields)) = response
            .metadata
            .get(apxm_core::constants::llm::apxm::FIELDS_HONORED_RECORD_KEY)
    {
        let honored: Vec<String> = fields
            .iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect();
        ctx.fields_honored.record(backend_name, honored);
    }

    let (prefill_ms, decode_ms) = response
        .timing
        .map(|t| (t.prefill_ms, t.decode_ms))
        .unwrap_or((total_ms, 0.0));
    ctx.timing_tracker.record(node.id, prefill_ms, decode_ms);

    // Record token usage in accountant
    {
        let flow_name = node
            .attributes
            .get(graph_attrs::FLOW_NAME)
            .and_then(|v| v.as_string());
        let agent_name = ctx.current_agent.as_ref().map(|a| a.name.as_str());
        ctx.token_accountant.record_usage(
            node.id,
            &response.usage,
            flow_name.map(|s| s.as_str()),
            agent_name,
        );
        // also feed the process-wide meter so `/v1/generate` (which
        // bypasses this executor path entirely) and the executor path are
        // observed through one shared counter.
        crate::executor::token_accounting::global_meter().record_usage(
            node.id,
            &response.usage,
            flow_name.map(|s| s.as_str()),
            agent_name,
        );
        if let Some(emitter) = &ctx.event_emitter {
            emitter.emit_token_usage(
                node.id,
                response.usage.input_tokens,
                response.usage.output_tokens,
            );
        }
    }

    let content = response.content;

    // Store in memoization cache if deterministic
    if let Some(key) = memo_key {
        // Use per-op TTL based on operation type
        let ttl = request.operation_type.as_ref().map(MemoCache::ttl_for_op);

        ctx.response_cache.put_with_ttl(
            key,
            content.clone(),
            response.usage.input_tokens,
            response.usage.output_tokens,
            response.model.clone(),
            ttl,
        );
    }

    apxm_llm!(info,
     execution_id = %ctx.execution_id,
     mode = mode_name,
     response_len = content.len(),
     tokens_in = response.usage.input_tokens,
     tokens_out = response.usage.output_tokens,
     "LLM response received"
    );

    apxm_llm!(trace,
     execution_id = %ctx.execution_id,
     mode = mode_name,
     raw_response = %content,
     "LLM model response content"
    );

    // Process response based on mode
    match mode {
        LlmMode::Ask | LlmMode::Think => {
            // Record LLM result in AAM
            let label = TransitionLabel::operation(node.id, node.op_type);
            ctx.aam.set_belief(
                format!(
                    "{}{}:{}",
                    belief_keys::LLM_RESULT_PREFIX,
                    mode_name,
                    node.id
                ),
                Value::String(content.chars().take(200).collect::<String>()),
                label,
            );
            Ok(Value::String(content))
        }
        LlmMode::Reason => {
            // Try to parse as structured output for Reason
            if let Ok(structured) = parse_structured_output(&content) {
                process_structured_output(ctx, node, structured, enable_inner_plan, bind_outputs)
                    .await
            } else {
                // Fall back to plain text response
                Ok(Value::String(content))
            }
        }
    }
}

/// Accept only the compiler-written memoization guard.
///
/// The compiler rewrites this field after it has checked effects, authority,
/// replay, dependencies, and configured backend evidence. A runtime default or
/// author-provided omission cannot enable response reuse.
fn compiler_stamped_memoization(node: &Node) -> bool {
    node.attributes
        .get(graph_attrs::MEMOIZABLE)
        .and_then(|value| value.as_bool())
        == Some(true)
}

/// Build cache identity only from complete configured request, implementation,
/// and authority evidence.
///
/// A request without an explicit model, a registered backend, or a declared
/// model profile cannot prove the implementation/authority boundary required
/// for response reuse, so memoization remains disabled for that call.
fn memoization_identity(
    ctx: &ExecutionContext,
    request: &LLMRequest,
) -> Option<MemoizationIdentity> {
    let model_profile = request.model_profile.as_deref()?.trim();
    if model_profile.is_empty() {
        return None;
    }
    let requested_model = request.model.as_deref()?.trim();
    if requested_model.is_empty() {
        return None;
    }
    let backend_name = ctx.llm_registry.resolve_backend_name(request).ok()?;
    let backend = ctx.llm_registry.get_backend(&backend_name)?;
    let configured_model = backend.model().trim();
    if configured_model.is_empty() {
        return None;
    }
    let request_id = memoization_request_identity(request)?;
    let authority_id = memoization_authority_identity(ctx, model_profile);
    Some(MemoizationIdentity {
        request_id,
        implementation_id: canonical_json_identity(serde_json::json!({
            "registry_backend": backend_name,
            "backend_name": backend.name(),
            "backend_model": configured_model,
            "requested_backend": &request.backend,
            "requested_model": requested_model,
        })),
        authority_id,
    })
}

/// Capture every request field that can change backend-visible generation.
///
/// Correlation-only ids are excluded: they associate observability records but
/// do not alter a deterministic model result. Provider routing, generation
/// controls, request metadata, schemas, tools, messages, and extra body data
/// remain part of the identity.
fn memoization_request_identity(request: &LLMRequest) -> Option<String> {
    if request.temperature != 0.0 {
        return None;
    }
    let metadata = request
        .metadata
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect::<BTreeMap<_, _>>();
    let messages = serde_json::to_value(&request.messages).ok()?;
    let tools = serde_json::to_value(&request.tools).ok()?;
    let tool_choice = serde_json::to_value(&request.tool_choice).ok()?;

    Some(canonical_json_identity(serde_json::json!({
        "prompt": &request.prompt,
        "messages": messages,
        "system_prompt": &request.system_prompt,
        "temperature": request.temperature,
        "max_tokens": request.max_tokens,
        "top_p": request.top_p,
        "frequency_penalty": request.frequency_penalty,
        "presence_penalty": request.presence_penalty,
        "stop_sequences": &request.stop_sequences,
        "output_schema": &request.output_schema,
        "thinking_token_budget": request.thinking_token_budget,
        "enable_thinking": request.enable_thinking,
        "metadata": metadata,
        "backend": &request.backend,
        "model": &request.model,
        "model_profile": &request.model_profile,
        "operation_type": request.operation_type.as_ref().map(|operation| format!("{operation:?}")),
        "tools": tools,
        "tool_choice": tool_choice,
        "extra_body": &request.extra_body,
    })))
}

/// Include all execution authority that can change the permission envelope for
/// a request even though compiler-proven memoized LLM nodes cannot use tools.
fn memoization_authority_identity(ctx: &ExecutionContext, model_profile: &str) -> String {
    canonical_json_identity(serde_json::json!({
        "model_profile": model_profile,
        "capability_grants": ctx.metadata.get(crate::metadata_keys::CAPABILITY_GRANTS),
        "side_effect_policy": ctx.metadata.get(crate::metadata_keys::SIDE_EFFECT_POLICY),
    }))
}

/// Render JSON with object keys ordered recursively so equivalent request
/// metadata cannot produce different cache identities because of map order.
fn canonical_json_identity(value: JsonValue) -> String {
    match value {
        JsonValue::Null => "null".to_string(),
        JsonValue::Bool(value) => value.to_string(),
        JsonValue::Number(value) => value.to_string(),
        JsonValue::String(value) => serde_json::to_string(&value).unwrap_or_default(),
        JsonValue::Array(values) => format!(
            "[{}]",
            values
                .into_iter()
                .map(canonical_json_identity)
                .collect::<Vec<_>>()
                .join(",")
        ),
        JsonValue::Object(values) => {
            let mut entries = values.into_iter().collect::<Vec<_>>();
            entries.sort_unstable_by(|left, right| left.0.cmp(&right.0));
            format!(
                "{{{}}}",
                entries
                    .into_iter()
                    .map(|(key, value)| format!(
                        "{}:{}",
                        serde_json::to_string(&key).unwrap_or_default(),
                        canonical_json_identity(value)
                    ))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
    }
}
