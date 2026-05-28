//! Function-calling tool registry lookup, parallel dispatch, and ASK tool loop.

use super::{
    ExecutionContext, charge_tokens, copy_llm_request_routing, resolve_global_token_budget,
};
use apxm_backends::{LLMRequest, ToolDefinition};
use apxm_core::apxm_llm;
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::constants::runtime::belief_keys;
use apxm_core::error::RuntimeError;
use apxm_core::types::execution::Node;
use apxm_core::types::values::Value;
use apxm_core::types::{ToolCall, ToolResult};
use std::collections::HashMap;

use super::super::{Result, execute_llm_request_for_node};

/// Default maximum number of tool loop iterations to prevent infinite loops.
/// Can be overridden per-node via the `max_tool_iterations` attribute.
pub(super) const DEFAULT_MAX_TOOL_ITERATIONS: usize = 10;
const DEFAULT_MAX_PARALLEL_TOOL_CALLS: usize = 8;
const HARD_MAX_PARALLEL_TOOL_CALLS: usize = 64;
const MAX_PARALLEL_TOOL_CALLS_ENV: &str = "APXM_RUNTIME_MAX_PARALLEL_TOOL_CALLS";

/// Get tool definitions from the capability system for LLM requests
fn get_tool_definitions_from_capabilities(ctx: &ExecutionContext) -> Vec<ToolDefinition> {
    let mut tools: Vec<ToolDefinition> = ctx
        .capability_system
        .list_capabilities()
        .into_iter()
        .map(|meta| ToolDefinition::new(&meta.name, &meta.description, meta.parameters_schema))
        .collect();
    if let Some(bridge) = ctx.python_tool_bridge.as_ref() {
        tools.extend(bridge.descriptors().map(|descriptor| {
            ToolDefinition::new(
                &descriptor.name,
                &descriptor.description,
                descriptor.schema.clone(),
            )
        }));
    }
    tools
}

/// Get tool definitions from the capability system filtered by group.
fn get_tool_definitions_from_groups(
    ctx: &ExecutionContext,
    groups: &[String],
) -> Vec<ToolDefinition> {
    ctx.capability_system
        .list_capabilities_by_groups(groups)
        .into_iter()
        .map(|meta| ToolDefinition::new(&meta.name, &meta.description, meta.parameters_schema))
        .collect()
}

/// Get specific tools by name from the capability system
fn get_tools_by_names(ctx: &ExecutionContext, names: &[String]) -> Vec<ToolDefinition> {
    names
        .iter()
        .filter_map(|name| {
            if let Some(meta) = ctx.capability_system.get_metadata(name) {
                return Some(ToolDefinition::new(
                    &meta.name,
                    &meta.description,
                    meta.parameters_schema,
                ));
            }
            ctx.python_tool_bridge
                .as_ref()
                .and_then(|bridge| bridge.registry().resolve(name))
                .map(|descriptor| {
                    ToolDefinition::new(
                        &descriptor.name,
                        &descriptor.description,
                        descriptor.schema.clone(),
                    )
                })
        })
        .collect()
}

fn parse_string_array_attr(node: &Node, attr_name: &str) -> Option<Vec<String>> {
    node.attributes
        .get(attr_name)
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_string().map(ToString::to_string))
                .collect()
        })
}

pub(super) fn resolve_ask_tools(ctx: &ExecutionContext, node: &Node) -> Vec<ToolDefinition> {
    let tool_names = parse_string_array_attr(node, graph_attrs::TOOLS);
    let tool_groups = parse_string_array_attr(node, graph_attrs::TOOL_GROUPS);
    let tools_enabled_all = node
        .attributes
        .get(graph_attrs::TOOLS_ENABLED)
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    match (tool_names, tool_groups, tools_enabled_all) {
        (Some(names), _, _) if !names.is_empty() => get_tools_by_names(ctx, &names),
        (_, Some(groups), true) if !groups.is_empty() => {
            get_tool_definitions_from_groups(ctx, &groups)
        }
        (_, _, true) => get_tool_definitions_from_capabilities(ctx),
        _ => vec![],
    }
}

/// Execute a single tool call.
///
/// Dispatch order:
///   1. Python tool bridge (if attached and the tool name is registered).
///   2. Rust capability system (built-ins and Rust-registered capabilities).
///
/// LLM-issued `tool_calls` carry only a name + JSON args, so the runtime
/// resolves them by name; the bridge's manifest (loaded from the artifact's
/// `python_tools` section) is the authoritative source for which names are
/// Python-backed.
async fn execute_tool_call(ctx: &ExecutionContext, tool_call: &ToolCall) -> ToolResult {
    apxm_llm!(debug,
        execution_id = %ctx.execution_id,
        tool_name = %tool_call.name,
        tool_id = %tool_call.id,
        "Executing tool call"
    );

    let args: HashMap<String, Value> = match &tool_call.args {
        serde_json::Value::Object(obj) => obj
            .iter()
            .map(|(k, v)| (k.clone(), json_to_value(v)))
            .collect(),
        _ => HashMap::new(),
    };

    if let Some(emitter) = &ctx.event_emitter {
        emitter.emit_tool_start(&tool_call.name, &args);
    }

    if let Some(bridge) = ctx.python_tool_bridge.as_ref() {
        if bridge.has_tool(&tool_call.name) {
            let timeout = std::time::Duration::from_millis(
                apxm_core::constants::defaults::DEFAULT_TIMEOUT_MS,
            );
            let json_args = tool_call.args.clone();
            return match bridge.call(&tool_call.name, json_args, timeout).await {
                Ok(json_result) => {
                    let content = match json_result {
                        serde_json::Value::String(s) => s,
                        other => other.to_string(),
                    };
                    if let Some(emitter) = &ctx.event_emitter {
                        emitter.emit_tool_end(&tool_call.name, &Value::String(content.clone()));
                    }
                    apxm_llm!(info,
                        execution_id = %ctx.execution_id,
                        tool_name = %tool_call.name,
                        "Python tool call succeeded"
                    );
                    ToolResult::success(&tool_call.id, content)
                }
                Err(e) => {
                    if let Some(emitter) = &ctx.event_emitter {
                        emitter.emit_tool_end(&tool_call.name, &Value::String(e.to_string()));
                    }
                    apxm_llm!(warn,
                        execution_id = %ctx.execution_id,
                        tool_name = %tool_call.name,
                        error = %e,
                        "Python tool call failed"
                    );
                    ToolResult::error(&tool_call.id, e.to_string())
                }
            };
        }
    }

    match ctx.capability_system.invoke(&tool_call.name, args).await {
        Ok(result) => {
            let content = match result {
                Value::String(s) => s,
                other => other.to_string(),
            };
            if let Some(emitter) = &ctx.event_emitter {
                emitter.emit_tool_end(&tool_call.name, &Value::String(content.clone()));
            }
            apxm_llm!(info,
                execution_id = %ctx.execution_id,
                tool_name = %tool_call.name,
                "Tool call succeeded"
            );
            ToolResult::success(&tool_call.id, content)
        }
        Err(e) => {
            if let Some(emitter) = &ctx.event_emitter {
                emitter.emit_tool_end(&tool_call.name, &Value::String(e.to_string()));
            }
            apxm_llm!(warn,
                execution_id = %ctx.execution_id,
                tool_name = %tool_call.name,
                error = %e,
                "Tool call failed"
            );
            ToolResult::error(&tool_call.id, e.to_string())
        }
    }
}

/// Per-tool-name write locks for concurrent tool dispatch.
///
/// Read-only tools run without locking. Write tools acquire a write lock
/// keyed by tool name so concurrent writes to the same tool are serialized
/// while independent tools execute in parallel.
static TOOL_WRITE_LOCKS: once_cell::sync::Lazy<
    dashmap::DashMap<String, std::sync::Arc<tokio::sync::RwLock<()>>>,
> = once_cell::sync::Lazy::new(dashmap::DashMap::new);

/// Execute multiple tool calls concurrently, preserving result order.
///
/// Read-only tools (according to `CapabilityMetadata::read_only`) run in full
/// parallel. Write tools acquire a per-tool-name `RwLock` so that:
/// - Multiple read-only calls execute simultaneously.
/// - Write calls on the same tool name are serialized.
/// - Write calls on different tool names run in parallel.
async fn execute_tool_calls_parallel(
    ctx: &ExecutionContext,
    tool_calls: &[ToolCall],
) -> Vec<ToolResult> {
    if tool_calls.len() == 1 {
        return vec![execute_tool_call(ctx, &tool_calls[0]).await];
    }

    let max_parallel = max_parallel_tool_calls(tool_calls.len());
    let batches = tool_calls.len().div_ceil(max_parallel);
    apxm_llm!(info,
        execution_id = %ctx.execution_id,
        tool_count = tool_calls.len(),
        max_parallel = max_parallel,
        batches = batches,
        "Dispatching tool calls in parallel"
    );

    let mut results = Vec::with_capacity(tool_calls.len());
    for batch in tool_calls.chunks(max_parallel) {
        results.extend(execute_tool_call_batch(ctx, batch).await);
    }
    results
}

async fn execute_tool_call_batch(
    ctx: &ExecutionContext,
    tool_calls: &[ToolCall],
) -> Vec<ToolResult> {
    let futures: Vec<_> = tool_calls
        .iter()
        .map(|tc| {
            let is_read_only = ctx.capability_system.is_read_only(&tc.name);
            async move {
                if is_read_only {
                    execute_tool_call(ctx, tc).await
                } else {
                    let lock = TOOL_WRITE_LOCKS
                        .entry(tc.name.clone())
                        .or_insert_with(|| std::sync::Arc::new(tokio::sync::RwLock::new(())))
                        .clone();
                    let _guard = lock.write().await;
                    execute_tool_call(ctx, tc).await
                }
            }
        })
        .collect();

    futures::future::join_all(futures).await
}

fn max_parallel_tool_calls(tool_count: usize) -> usize {
    let configured = std::env::var(MAX_PARALLEL_TOOL_CALLS_ENV)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_MAX_PARALLEL_TOOL_CALLS);
    clamp_tool_call_parallelism(tool_count, configured)
}

fn clamp_tool_call_parallelism(tool_count: usize, configured: usize) -> usize {
    if tool_count == 0 {
        0
    } else {
        configured
            .clamp(1, HARD_MAX_PARALLEL_TOOL_CALLS)
            .min(tool_count)
    }
}

/// Convert serde_json::Value to apxm_core Value
pub(super) fn json_to_value(json: &serde_json::Value) -> Value {
    match json {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(b) => Value::Bool(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Number(apxm_core::types::values::Number::Integer(i))
            } else if let Some(f) = n.as_f64() {
                Value::Number(apxm_core::types::values::Number::Float(f))
            } else {
                Value::Null
            }
        }
        serde_json::Value::String(s) => Value::String(s.clone()),
        serde_json::Value::Array(arr) => Value::Array(arr.iter().map(json_to_value).collect()),
        serde_json::Value::Object(obj) => Value::Object(
            obj.iter()
                .map(|(k, v)| (k.clone(), json_to_value(v)))
                .collect(),
        ),
    }
}

/// Format tool results as a message for the LLM
pub(super) fn format_tool_results_message(results: &[ToolResult]) -> String {
    results
        .iter()
        .map(|r| {
            if r.success {
                format!(
                    "<tool_result id=\"{}\">\n{}\n</tool_result>",
                    r.tool_call_id, r.content
                )
            } else {
                format!(
                    "<tool_error id=\"{}\">\n{}\n</tool_error>",
                    r.tool_call_id, r.content
                )
            }
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Execute Ask operation with tool loop
///
/// This implements the tool use cycle:
/// 1. Send prompt + tool schemas to LLM
/// 2. LLM returns tool_calls (or text)
/// 3. Execute tool calls via CapabilitySystem
/// 4. Feed results back to LLM
/// 5. Repeat until LLM returns text (no tool calls)
pub(super) async fn execute_ask_with_tools(
    ctx: &ExecutionContext,
    node: &Node,
    initial_request: &LLMRequest,
) -> Result<Value> {
    let max_iterations = node
        .attributes
        .get(graph_attrs::MAX_TOOL_ITERATIONS)
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
        .unwrap_or(DEFAULT_MAX_TOOL_ITERATIONS);

    let mut current_request = initial_request.clone();
    let mut accumulated_tool_results: Vec<ToolResult> = Vec::new();
    let mut total_input_tokens = 0usize;
    let mut total_output_tokens = 0usize;
    let mut total_prefill_ms = 0.0_f64;
    let mut total_decode_ms = 0.0_f64;

    for iteration in 0..max_iterations {
        if ctx.cancellation_token.is_cancelled() {
            return Err(RuntimeError::SchedulerCancelled);
        }

        apxm_llm!(debug,
            execution_id = %ctx.execution_id,
            iteration = iteration,
            prompt_len = current_request.prompt.len(),
            tool_count = current_request.tools.as_ref().map(|t| t.len()).unwrap_or(0),
            "Sending ASK request with tools"
        );

        let llm_start = std::time::Instant::now();
        let response = execute_llm_request_for_node(ctx, node, "ASK", &current_request).await?;
        let iter_total_ms = llm_start.elapsed().as_secs_f64() * 1000.0;
        let (iter_prefill, iter_decode) = response
            .timing
            .map(|t| (t.prefill_ms, t.decode_ms))
            .unwrap_or((iter_total_ms, 0.0));
        total_prefill_ms += iter_prefill;
        total_decode_ms += iter_decode;
        charge_tokens(
            ctx,
            resolve_global_token_budget(ctx),
            response.usage.total_tokens,
        )?;

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

        total_input_tokens += response.usage.input_tokens;
        total_output_tokens += response.usage.output_tokens;

        apxm_llm!(info,
            execution_id = %ctx.execution_id,
            iteration = iteration,
            response_len = response.content.len(),
            tool_calls = response.tool_calls.len(),
            tokens_in = response.usage.input_tokens,
            tokens_out = response.usage.output_tokens,
            "ASK response received"
        );

        if response.tool_calls.is_empty() {
            apxm_llm!(info,
                execution_id = %ctx.execution_id,
                iterations = iteration + 1,
                total_tokens_in = total_input_tokens,
                total_tokens_out = total_output_tokens,
                tools_invoked = accumulated_tool_results.len(),
                "ASK tool loop completed"
            );
            ctx.timing_tracker
                .record(node.id, total_prefill_ms, total_decode_ms);
            return Ok(Value::String(response.content));
        }

        let tool_results = execute_tool_calls_parallel(ctx, &response.tool_calls).await;

        if !tool_results.is_empty() {
            let results_value = Value::Array(
                tool_results
                    .iter()
                    .map(|r| {
                        let mut obj = HashMap::new();
                        obj.insert(
                            "tool_call_id".to_string(),
                            Value::String(r.tool_call_id.clone()),
                        );
                        obj.insert("content".to_string(), Value::String(r.content.clone()));
                        obj.insert("success".to_string(), Value::Bool(r.success));
                        Value::Object(obj)
                    })
                    .collect(),
            );

            ctx.memory
                .write_scoped(
                    crate::memory::MemorySpace::Stm,
                    ctx.scope_id(),
                    format!(
                        "{}{}:{}",
                        belief_keys::TOOL_RESULTS_PREFIX,
                        ctx.execution_id,
                        iteration
                    ),
                    results_value,
                )
                .await
                .ok();
        }

        accumulated_tool_results.extend(tool_results.clone());

        let tool_results_message = format_tool_results_message(&tool_results);
        let continuation_prompt = format!(
            "{}\n\n{}\n\nBased on the tool results above, please continue.",
            current_request.prompt, tool_results_message
        );

        current_request = copy_llm_request_routing(
            LLMRequest::new(continuation_prompt)
                .with_system_prompt(current_request.system_prompt.clone().unwrap_or_default())
                .with_temperature(current_request.temperature),
            initial_request,
        );

        if let Some(tools) = &initial_request.tools {
            current_request = current_request.with_tools(tools.clone());
        }
        if let Some(choice) = &initial_request.tool_choice {
            current_request = current_request.with_tool_choice(choice.clone());
        }
        if let Some(hints) = &initial_request.apxm_hints {
            current_request = current_request.with_apxm_hints(hints.clone());
        }
    }

    apxm_llm!(warn,
        execution_id = %ctx.execution_id,
        max_iterations = max_iterations,
        "ASK tool loop exceeded max iterations"
    );
    ctx.timing_tracker
        .record(node.id, total_prefill_ms, total_decode_ms);

    Err(RuntimeError::LLM {
        message: format!(
            "Tool loop exceeded maximum iterations ({}). Last {} tool calls executed.",
            max_iterations,
            accumulated_tool_results.len()
        ),
        backend: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aam::Aam;
    use crate::capability::CapabilitySystem;
    use crate::capability::builtins::{ReadCapability, SearchWebCapability};
    use crate::memory::{MemoryConfig, MemorySystem};
    use apxm_core::types::operations::AISOperationType;
    use std::sync::Arc;

    async fn ctx_with_grouped_tools() -> ExecutionContext {
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
            Aam::new(),
        )
    }

    #[test]
    fn json_to_value_handles_primitives() {
        assert_eq!(
            json_to_value(&serde_json::json!("hello")),
            Value::String("hello".to_string())
        );
        assert!(matches!(
            json_to_value(&serde_json::json!(42)),
            Value::Number(_)
        ));
        assert_eq!(json_to_value(&serde_json::json!(true)), Value::Bool(true));
        assert_eq!(json_to_value(&serde_json::json!(null)), Value::Null);
        assert!(matches!(
            json_to_value(&serde_json::json!([1, 2, 3])),
            Value::Array(_)
        ));
        assert!(matches!(
            json_to_value(&serde_json::json!({"key": "value"})),
            Value::Object(_)
        ));
    }

    #[test]
    fn format_tool_results_message_separates_success_from_error() {
        let results = vec![
            ToolResult::success("call_1", "file1.txt\nfile2.txt"),
            ToolResult::error("call_2", "Permission denied"),
        ];
        let message = format_tool_results_message(&results);
        assert!(message.contains("<tool_result id=\"call_1\">"));
        assert!(message.contains("file1.txt"));
        assert!(message.contains("<tool_error id=\"call_2\">"));
        assert!(message.contains("Permission denied"));
    }

    #[tokio::test]
    async fn resolve_ask_tools_filters_by_group_when_enabled() {
        let ctx = ctx_with_grouped_tools().await;
        let mut node = Node::new(1, AISOperationType::Ask);
        node.attributes
            .insert(graph_attrs::TOOLS_ENABLED.to_string(), Value::Bool(true));
        node.attributes.insert(
            graph_attrs::TOOL_GROUPS.to_string(),
            Value::Array(vec![Value::String("web".to_string())]),
        );
        let names: Vec<String> = resolve_ask_tools(&ctx, &node)
            .into_iter()
            .map(|tool| tool.name)
            .collect();
        assert_eq!(names, vec!["search_web".to_string()]);
    }

    #[tokio::test]
    async fn resolve_ask_tools_explicit_names_override_groups() {
        let ctx = ctx_with_grouped_tools().await;
        let mut node = Node::new(1, AISOperationType::Ask);
        node.attributes
            .insert(graph_attrs::TOOLS_ENABLED.to_string(), Value::Bool(true));
        node.attributes.insert(
            graph_attrs::TOOL_GROUPS.to_string(),
            Value::Array(vec![Value::String("web".to_string())]),
        );
        node.attributes.insert(
            graph_attrs::TOOLS.to_string(),
            Value::Array(vec![Value::String("read".to_string())]),
        );
        let names: Vec<String> = resolve_ask_tools(&ctx, &node)
            .into_iter()
            .map(|tool| tool.name)
            .collect();
        assert_eq!(names, vec!["read".to_string()]);
    }

    #[tokio::test]
    async fn resolve_ask_tools_groups_do_not_enable_tools_by_themselves() {
        let ctx = ctx_with_grouped_tools().await;
        let mut node = Node::new(1, AISOperationType::Ask);
        node.attributes.insert(
            graph_attrs::TOOL_GROUPS.to_string(),
            Value::Array(vec![Value::String("web".to_string())]),
        );
        assert!(resolve_ask_tools(&ctx, &node).is_empty());
    }

    #[test]
    fn tool_call_parallelism_is_bounded_and_nonzero() {
        assert_eq!(clamp_tool_call_parallelism(0, 8), 0);
        assert_eq!(clamp_tool_call_parallelism(3, 8), 3);
        assert_eq!(clamp_tool_call_parallelism(100, 0), 1);
        assert_eq!(
            clamp_tool_call_parallelism(1000, usize::MAX),
            HARD_MAX_PARALLEL_TOOL_CALLS
        );
    }
}
