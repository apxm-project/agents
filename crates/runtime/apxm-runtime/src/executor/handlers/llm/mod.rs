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
    copy_llm_request_routing, execute_llm_request_for_node, get_optional_string_attribute,
    get_optional_u64_attribute, get_string_attribute,
    template::{input_names_from_node, render_named},
    warmup::{dispatch_warmup, should_dispatch_warmup},
};
use crate::aam::TransitionLabel;
use crate::executor::memoization::MemoCache;
use apxm_backends::llm::backends::vllm::attrs as vllm_attrs;
use apxm_backends::{LLMRequest, ToolChoice};
use apxm_core::apxm_llm;
use apxm_core::constants::{
    extra_body as extra_body_keys,
    graph::{attrs as graph_attrs, metadata as graph_meta},
    runtime::belief_keys,
};
use apxm_core::error::RuntimeError;
use apxm_core::types::operations::AISOperationType;
use apxm_core::types::{ApxmGraphHints, PriorityClass};
use serde_json::Value as JsonValue;

pub(super) mod pipeline;
pub(super) mod structured_output;
pub(super) mod tool_dispatch;

use pipeline::{
    charge_tokens, default_memoizable_for_backend, resolve_global_token_budget,
    resolve_node_output_token_limit,
};
use structured_output::{
    build_schema_retry_prompt, output_schema_from_node, parse_structured_output,
    process_structured_output, validate_against_output_schema,
};
use tool_dispatch::{execute_ask_with_tools, resolve_ask_tools};

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

fn resolve_system_prompt(ctx: &ExecutionContext, node: &Node, mode: LlmMode) -> Result<String> {
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
    let prompt = get_optional_string_attribute(node, graph_attrs::SYSTEM_PROMPT)?
        .or_else(|| config_instruction.cloned())
        .or_else(|| apxm_backends::render_prompt(template_name, &serde_json::json!({})).ok())
        .unwrap_or_else(|| fallback.to_string());
    Ok(prompt)
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
///   1. Explicit `vllm_cache_salt` node attribute — author intent always wins.
///      A value of `"none"` (or empty) disables salting entirely.
///   2. Compiler-stamped `shared_prefix_group` — the SharedPrefixAnalysis
///      pass marks sibling nodes that share a bit-identical leading prompt.
///      Salting by `{graph_id}:{group}` lets the vLLM prefix cache survive
///      across executions of the same graph for grouped nodes, while the
///      benchmark harness can still keep ungrouped nodes execution-isolated
///      via `APXM_VLLM_CACHE_SALT=execution`.
///   3. Env var fallback (`APXM_VLLM_CACHE_SALT`) — harness iteration
///      isolation for ungrouped nodes.
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

    let reuse_group = match get_optional_string_attribute(node, graph_attrs::REUSE_GROUP)? {
        Some(g) => Some(g),
        None => get_optional_string_attribute(node, graph_attrs::REUSE_GROUP_LEGACY)?,
    }
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

    // Build prompt by named substitution. The compiler validator guarantees
    // that any non-empty input chain has a matching `input_names` array and
    // that every `{name}` in the template resolves against it.
    let prompt = if inputs.is_empty() {
        base_prompt.clone()
    } else {
        let input_names = input_names_from_node(node);
        render_named(&base_prompt, &inputs, &input_names)?
    };

    let mut request = apply_llm_request_routing_from_node(
        LLMRequest::new(prompt.clone()).with_operation_type(node.op_type),
        node,
    )?;

    // Apply mode-specific configuration
    if mode == LlmMode::Think
        && let Some(budget_tokens) = budget
    {
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

    let system_prompt = resolve_system_prompt(ctx, node, mode)?;
    request = request.with_system_prompt(system_prompt);

    // Tool configuration (Ask mode only). Tools are OPT-IN: a node only
    // attaches tools when it explicitly opts in via TOOLS (named list) or
    // TOOLS_ENABLED=true (use all registered capabilities). The default is
    // no tools — matches OpenAI/LangChain/CrewAI/PydanticAI behavior.
    if mode == LlmMode::Ask {
        let tools = resolve_ask_tools(ctx, node);

        if !tools.is_empty() {
            // Hard-fail if the resolved backend can't accept tool_choice="auto".
            // Some OpenAI-compatible servers reject it; surface a clear,
            // actionable error instead of silently dropping tools. Configure
            // `auto_tool_choice = false` in `~/.apxm/config.toml` for such
            // servers.
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
            if !backend.supports_auto_tool_choice() {
                return Err(RuntimeError::LLM {
                    message: format!(
                        "Backend '{}' does not support tool_choice=\"auto\". \
                         Either remove tool usage from this node, configure \
                         `auto_tool_choice = false` for this backend in \
                         `~/.apxm/config.toml`, or enable automatic tool choice \
                         in the registered backend adapter.",
                        backend_name
                    ),
                    backend: Some(backend_name),
                });
            }

            apxm_llm!(debug,
                execution_id = %ctx.execution_id,
                tool_count = tools.len(),
                "Attaching tools to ASK request"
            );
            request = request.with_tools(tools).with_tool_choice(ToolChoice::Auto);
        }
    }

    request = apply_vllm_request_overrides_from_node(ctx, node, request)?;

    // Attach APXM graph hints for graph-aware backends.
    request = attach_graph_hints(ctx, node, request);

    if let Some(estimated_prefix_tokens) = should_dispatch_warmup(ctx, node, &request) {
        dispatch_warmup(ctx, node.id, mode_name, &request, estimated_prefix_tokens).await?;
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

        match execute_llm_once(ctx, node, &request, mode, enable_inner_plan, bind_outputs).await {
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
) -> Result<Value> {
    let mode_name = mode.name();

    // For Ask mode with tools, use the tool loop
    if mode == LlmMode::Ask && request.has_tools() {
        return execute_ask_with_tools(ctx, node, request).await;
    }

    let resolved_backend = get_optional_string_attribute(node, graph_attrs::BACKEND)?;
    let memoizable = node
        .attributes
        .get(graph_attrs::MEMOIZABLE)
        .and_then(|v| v.as_bool())
        .unwrap_or_else(|| default_memoizable_for_backend(resolved_backend.as_deref()));

    // Check memoization cache for deterministic (temperature=0) calls
    let memo_key = if memoizable {
        // Extract tool names if present
        let tools_list: Option<Vec<String>> = request
            .tools
            .as_ref()
            .map(|tools| tools.iter().map(|t| t.name.clone()).collect());

        // Get output schema if present
        let output_schema = output_schema_from_node(node)?;
        let output_schema_str = output_schema.as_ref().map(|s| s.to_string());

        MemoCache::compute_key(
            &request.prompt,
            request.system_prompt.as_deref(),
            request.model.as_deref(),
            request.temperature,
            tools_list.as_deref(),
            output_schema_str.as_deref(),
        )
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
    let response = execute_llm_request_for_node(ctx, node, mode_name, request).await?;
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

    charge_tokens(
        ctx,
        resolve_global_token_budget(ctx),
        response.usage.total_tokens,
    )?;

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
        let ttl = request
            .operation_type
            .as_ref()
            .map(|op| MemoCache::ttl_for_op(op));

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::CapabilitySystem;
    use crate::capability::builtins::{ReadCapability, SearchWebCapability};
    use crate::memory::{MemoryConfig, MemorySystem};
    use std::sync::Arc;
    use std::sync::atomic::Ordering;

    async fn test_ctx_with_grouped_tools() -> ExecutionContext {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .expect("memory"),
        );
        let capability_system = Arc::new(CapabilitySystem::new());
        capability_system
            .register(Arc::new(ReadCapability::new()))
            .expect("register read");
        capability_system
            .register(Arc::new(SearchWebCapability::new()))
            .expect("register web");

        ExecutionContext::new(
            memory,
            Arc::new(apxm_backends::LLMRegistry::new()),
            capability_system,
            crate::aam::Aam::new(),
        )
    }

    /// Scope guard for a process-global env var. Captures the prior value
    /// on construction and restores (or unsets) it on drop, so cache-salt
    /// tests can assert one value without leaking it to sibling tests.
    struct EnvVarGuard {
        key: &'static str,
        prev: Option<String>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let prev = std::env::var(key).ok();
            #[allow(unsafe_code)]
            unsafe {
                std::env::set_var(key, value);
            }
            Self { key, prev }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            #[allow(unsafe_code)]
            unsafe {
                match self.prev.take() {
                    Some(v) => std::env::set_var(self.key, v),
                    None => std::env::remove_var(self.key),
                }
            }
        }
    }

    #[test]
    fn test_llm_mode_from_op_type() {
        assert_eq!(LlmMode::from(&AISOperationType::Ask), LlmMode::Ask);
        assert_eq!(LlmMode::from(&AISOperationType::Think), LlmMode::Think);
        assert_eq!(LlmMode::from(&AISOperationType::Reason), LlmMode::Reason);
    }

    #[test]
    fn test_node_token_budget_lowers_to_output_limit() {
        let mut node = Node::new(7, AISOperationType::Ask);
        node.attributes.insert(
            graph_attrs::TOKEN_BUDGET.to_string(),
            Value::Number(apxm_core::types::values::Number::Integer(128)),
        );

        assert_eq!(resolve_node_output_token_limit(&node).unwrap(), Some(128));
    }

    #[tokio::test]
    async fn test_node_token_budget_does_not_create_global_charge_limit() {
        let ctx = test_ctx_with_grouped_tools().await;
        let mut node = Node::new(7, AISOperationType::Ask);
        node.attributes.insert(
            graph_attrs::TOKEN_BUDGET.to_string(),
            Value::Number(apxm_core::types::values::Number::Integer(128)),
        );

        assert_eq!(resolve_node_output_token_limit(&node).unwrap(), Some(128));
        assert_eq!(resolve_global_token_budget(&ctx), None);
        charge_tokens(&ctx, resolve_global_token_budget(&ctx), 10_000).unwrap();
        assert_eq!(ctx.consumed_tokens.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn test_attach_graph_hints_uses_context_graph_id() {
        let ctx = test_ctx_with_grouped_tools()
            .await
            .with_execution_id("exec-under-test".to_string())
            .with_graph_id("registered-graph-id".to_string());
        let mut node = Node::new(42, AISOperationType::Ask);
        node.metadata.name = Some("ask_node".to_string());
        node.metadata.priority = 95;

        let request = attach_graph_hints(&ctx, &node, LLMRequest::new("prompt"));
        let hints = request.apxm_hints.expect("graph hints should be attached");

        assert_eq!(hints.graph_id.as_deref(), Some("registered-graph-id"));
        assert_eq!(hints.execution_id.as_deref(), Some("exec-under-test"));
        assert_eq!(hints.node_id, Some(42));
        assert_eq!(hints.node_name.as_deref(), Some("ask_node"));
        assert_eq!(hints.priority_class, Some(PriorityClass::CriticalPath));
    }

    #[tokio::test]
    async fn test_attach_graph_hints_prefers_dispatch_ir_when_present() {
        use apxm_core::types::values::Number;
        use apxm_core::types::{DependencyType, Edge, ExecutionDag};

        let direct_ctx = test_ctx_with_grouped_tools()
            .await
            .with_execution_id("exec-dispatch".to_string())
            .with_graph_id("graph-dispatch".to_string());
        let dispatch_ctx = test_ctx_with_grouped_tools()
            .await
            .with_execution_id("exec-dispatch".to_string())
            .with_graph_id("graph-dispatch".to_string());

        let mut root = Node::new(42, AISOperationType::Ask);
        root.metadata.name = Some("ask_node".to_string());
        root.metadata.priority = 95;
        root.attributes.insert(
            graph_attrs::REUSE_GROUP.to_string(),
            Value::String("shared-prefix".to_string()),
        );
        root.attributes.insert(
            graph_attrs::SHARED_PREFIX_EST_TOKENS.to_string(),
            Value::Number(Number::Integer(256)),
        );
        root.attributes.insert(
            graph_attrs::DOWNSTREAM_NODES.to_string(),
            Value::Array(vec![Value::Number(Number::Integer(43))]),
        );
        let child = Node::new(43, AISOperationType::Ask);
        let mut dag = ExecutionDag::new();
        dag.add_node(root.clone()).unwrap();
        dag.add_node(child).unwrap();
        dag.add_edge(Edge::new(42, 43, 1, DependencyType::Data))
            .unwrap();

        let dispatch_ir = crate::graph_lifecycle::graph_dispatch_ir_from_dag(
            "graph-dispatch",
            "exec-dispatch",
            &dag,
        );
        dispatch_ctx.set_dispatch_ir_v1(dispatch_ir);

        let direct = attach_graph_hints(&direct_ctx, &root, LLMRequest::new("prompt"))
            .apxm_hints
            .expect("direct graph hints should be attached");
        let request = attach_graph_hints(&dispatch_ctx, &root, LLMRequest::new("prompt"));
        let hints = request.apxm_hints.expect("graph hints should be attached");

        assert_eq!(
            serde_json::to_value(&hints).unwrap(),
            serde_json::to_value(&direct).unwrap()
        );
        assert_eq!(hints.graph_id.as_deref(), Some("graph-dispatch"));
        assert_eq!(hints.execution_id.as_deref(), Some("exec-dispatch"));
        assert_eq!(hints.node_id, Some(42));
        assert_eq!(hints.node_name.as_deref(), Some("ask_node"));
        assert_eq!(hints.priority_class, Some(PriorityClass::CriticalPath));
        assert_eq!(hints.downstream_nodes, vec![43]);
        assert_eq!(hints.reuse_group.as_deref(), Some("shared-prefix"));
        assert_eq!(hints.compiler_hints.shared_prefix_est_tokens, Some(256));
    }

    #[tokio::test]
    async fn test_vllm_cache_salt_execution_override_sets_extra_body() {
        let ctx = test_ctx_with_grouped_tools()
            .await
            .with_execution_id("exec-cache-salt".to_string())
            .with_graph_id("graph-cache-salt".to_string());
        let mut node = Node::new(42, AISOperationType::Ask);
        node.attributes.insert(
            vllm_attrs::CACHE_SALT_ATTR.to_string(),
            Value::String(vllm_attrs::CACHE_SALT_EXECUTION.to_string()),
        );

        let request =
            apply_vllm_request_overrides_from_node(&ctx, &node, LLMRequest::new("prompt"))
                .expect("cache salt override should apply");

        assert_eq!(
            request
                .extra_body
                .as_ref()
                .and_then(|body| body.get(extra_body_keys::CACHE_SALT_KEY))
                .and_then(JsonValue::as_str),
            Some("exec-cache-salt")
        );
    }

    #[tokio::test]
    async fn test_vllm_cache_salt_preserves_existing_extra_body() {
        let ctx = test_ctx_with_grouped_tools()
            .await
            .with_execution_id("exec-cache-salt".to_string());
        let mut node = Node::new(42, AISOperationType::Ask);
        node.attributes.insert(
            vllm_attrs::CACHE_SALT_ATTR.to_string(),
            Value::String("literal-salt".to_string()),
        );

        let request = LLMRequest::new("prompt").with_extra_body(serde_json::json!({
            "priority": 5,
        }));
        let request = apply_vllm_request_overrides_from_node(&ctx, &node, request)
            .expect("cache salt override should apply");

        assert_eq!(
            request
                .extra_body
                .as_ref()
                .and_then(|body| body.get("priority"))
                .and_then(JsonValue::as_i64),
            Some(5)
        );
        assert_eq!(
            request
                .extra_body
                .as_ref()
                .and_then(|body| body.get(extra_body_keys::CACHE_SALT_KEY))
                .and_then(JsonValue::as_str),
            Some("literal-salt")
        );
    }

    #[tokio::test]
    async fn test_vllm_cache_salt_honors_compiler_shared_prefix_group() {
        // Compiler hint: when SharedPrefixAnalysis stamps `shared_prefix_group`
        // on a node, the runtime must salt by `{graph_id}:{group}` so sibling
        // nodes in that group reuse the vLLM prefix cache across executions
        // of the same graph — even when the harness has set the env var to
        // `execution` for iteration isolation of ungrouped nodes.
        let ctx = test_ctx_with_grouped_tools()
            .await
            .with_execution_id("exec-iter-9".to_string())
            .with_graph_id("review-synthesis-graph".to_string());
        let mut node = Node::new(4, AISOperationType::Ask);
        node.attributes.insert(
            graph_attrs::REUSE_GROUP.to_string(),
            Value::String("shared_prefix_analysis_0".to_string()),
        );

        // Exercise the precedence rule: compiler hint must win over the
        // env-var-driven `execution` salting the harness sets for ungrouped
        // iteration isolation. SAFETY: other env-mutating tests live under
        // their own serialization mutex in `vllm/attrs.rs`; this one runs
        // single-threaded in a tokio task.
        let _salt_env = EnvVarGuard::set(vllm_attrs::CACHE_SALT_ENV_VAR, "execution");

        let request =
            apply_vllm_request_overrides_from_node(&ctx, &node, LLMRequest::new("prompt"))
                .expect("compiler-hint salt override should apply");

        assert_eq!(
            request
                .extra_body
                .as_ref()
                .and_then(|body| body.get(extra_body_keys::CACHE_SALT_KEY))
                .and_then(JsonValue::as_str),
            Some("review-synthesis-graph:shared_prefix_analysis_0"),
            "shared_prefix_group must produce a graph-scoped salt, not the execution id"
        );
    }

    #[tokio::test]
    async fn test_vllm_cache_salt_honors_python_reuse_group_kwarg() {
        let ctx = test_ctx_with_grouped_tools()
            .await
            .with_execution_id("exec-iter-10".to_string())
            .with_graph_id("review-synthesis-graph".to_string());
        let mut node = Node::new(4, AISOperationType::Ask);
        node.attributes.insert(
            graph_attrs::REUSE_GROUP_LEGACY.to_string(),
            Value::String("apxm_review_council_shared_context".to_string()),
        );

        let _salt_env = EnvVarGuard::set(vllm_attrs::CACHE_SALT_ENV_VAR, "execution");

        let request =
            apply_vllm_request_overrides_from_node(&ctx, &node, LLMRequest::new("prompt"))
                .expect("python reuse_group kwarg should apply graph-scoped salt");

        assert_eq!(
            request
                .extra_body
                .as_ref()
                .and_then(|body| body.get(extra_body_keys::CACHE_SALT_KEY))
                .and_then(JsonValue::as_str),
            Some("review-synthesis-graph:apxm_review_council_shared_context"),
            "explicit Python `reuse_group=` must match compiler-stamped shared prefix salting"
        );
    }

    #[tokio::test]
    async fn test_vllm_cache_salt_explicit_attr_overrides_compiler_hint() {
        // Author intent (per-node `vllm_cache_salt = "execution"`) wins over
        // a compiler-stamped `shared_prefix_group`. This is the escape hatch
        // for workflows that explicitly want isolation even on grouped nodes.
        let ctx = test_ctx_with_grouped_tools()
            .await
            .with_execution_id("exec-isolated".to_string())
            .with_graph_id("graph-with-group".to_string());
        let mut node = Node::new(4, AISOperationType::Ask);
        node.attributes.insert(
            graph_attrs::REUSE_GROUP.to_string(),
            Value::String("shared_prefix_analysis_0".to_string()),
        );
        node.attributes.insert(
            vllm_attrs::CACHE_SALT_ATTR.to_string(),
            Value::String(vllm_attrs::CACHE_SALT_EXECUTION.to_string()),
        );

        let request =
            apply_vllm_request_overrides_from_node(&ctx, &node, LLMRequest::new("prompt"))
                .expect("explicit attr should override compiler hint");

        assert_eq!(
            request
                .extra_body
                .as_ref()
                .and_then(|body| body.get(extra_body_keys::CACHE_SALT_KEY))
                .and_then(JsonValue::as_str),
            Some("exec-isolated"),
            "explicit `vllm_cache_salt` must take precedence over `shared_prefix_group`"
        );
    }

    #[tokio::test]
    async fn test_vllm_cache_salt_explicit_none_disables_for_grouped_node() {
        // Workflow author can set `vllm_cache_salt = "none"` to disable
        // salting on a node that the compiler would otherwise group.
        let ctx = test_ctx_with_grouped_tools()
            .await
            .with_execution_id("exec-x".to_string())
            .with_graph_id("graph-x".to_string());
        let mut node = Node::new(4, AISOperationType::Ask);
        node.attributes.insert(
            graph_attrs::REUSE_GROUP.to_string(),
            Value::String("shared_prefix_analysis_0".to_string()),
        );
        node.attributes.insert(
            vllm_attrs::CACHE_SALT_ATTR.to_string(),
            Value::String(vllm_attrs::CACHE_SALT_NONE_LITERAL.to_string()),
        );

        let request =
            apply_vllm_request_overrides_from_node(&ctx, &node, LLMRequest::new("prompt"))
                .expect("explicit none should disable salting");

        assert!(
            request
                .extra_body
                .as_ref()
                .and_then(|body| body.get(extra_body_keys::CACHE_SALT_KEY))
                .is_none(),
            "explicit `vllm_cache_salt = none` must skip the salt entirely"
        );
    }
}
