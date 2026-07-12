//! Function-calling tool registry lookup, parallel dispatch, and ASK tool loop.

use super::{
    ExecutionContext, LlmAttemptOutput, attach_graph_hints, charge_tokens,
    copy_llm_request_routing, emit_llm_done, resolve_global_token_budget,
};
use apxm_backends::{LLMRequest, ToolChoice, ToolDefinition};
use apxm_core::apxm_llm;
use apxm_core::constants::capabilities;
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::constants::runtime::belief_keys;
use apxm_core::error::RuntimeError;
use apxm_core::types::execution::Node;
use apxm_core::types::operations::AISOperationType;
use apxm_core::types::values::Value;
use apxm_core::types::{ToolCall, ToolResult};
use std::collections::HashMap;

use super::super::{Result, apply_llm_request_routing_from_node, execute_llm_request_for_node};

/// Agent-callable tool that spawns a focused specialist sub-agent. The
/// converse/autonomous coordinator opts in via the `enable_delegate` node
/// attribute; calling several `delegate`s in one turn fans them out in
/// parallel (the in-ASK loop runs tool calls concurrently). This is the
/// runtime surface of apxm's sub-agent fan-out — graph ops like SPAWN_AGENT
/// are compile-time only, so converse agents reach delegation through here.
pub(crate) const DELEGATE_TOOL: &str = "delegate";

/// Default maximum number of tool loop iterations to prevent infinite loops.
/// Can be overridden per-node via the `max_tool_iterations` attribute.
pub(super) const DEFAULT_MAX_TOOL_ITERATIONS: usize = 10;

/// Get tool definitions from the capability system for LLM requests
fn get_tool_definitions_from_capabilities(ctx: &ExecutionContext) -> Vec<ToolDefinition> {
    let mut tools: Vec<ToolDefinition> = ctx
        .capability_system
        .list_capabilities()
        .into_iter()
        .map(|meta| ToolDefinition::new(&meta.name, &meta.description, meta.parameters_schema))
        .collect();
    if let Some(bridge) = ctx.python_handler_bridge.as_ref() {
        tools.extend(bridge.descriptors().map(|descriptor| {
            ToolDefinition::new(
                &descriptor.name,
                &descriptor.description,
                descriptor.schema.clone(),
            )
        }));
    }
    if let Some(bridge) = ctx.typescript_handler_bridge.as_ref() {
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
            ctx.python_handler_bridge
                .as_ref()
                .and_then(|bridge| bridge.registry().resolve(name))
                .map(|descriptor| {
                    ToolDefinition::new(
                        &descriptor.name,
                        &descriptor.description,
                        descriptor.schema.clone(),
                    )
                })
                .or_else(|| {
                    ctx.typescript_handler_bridge.as_ref().and_then(|bridge| {
                        bridge.registry().resolve(name).map(|descriptor| {
                            ToolDefinition::new(
                                &descriptor.name,
                                &descriptor.description,
                                descriptor.schema.clone(),
                            )
                        })
                    })
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

pub(crate) fn resolve_ask_tools(ctx: &ExecutionContext, node: &Node) -> Vec<ToolDefinition> {
    let tool_names = parse_string_array_attr(node, graph_attrs::TOOLS);
    let capability_groups = parse_string_array_attr(node, graph_attrs::CAPABILITY_GROUPS);
    let tools_enabled_all = node
        .attributes
        .get(graph_attrs::TOOLS_ENABLED)
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let mut tools = Vec::new();
    let mut explicitly_scoped = false;
    if let Some(names) = tool_names.as_deref()
        && !names.is_empty()
    {
        explicitly_scoped = true;
        tools.extend(get_tools_by_names(ctx, names));
    }
    // Naming tool groups is additive with explicit tools. A conversational agent
    // commonly has both Python tools and a capability group such as `skills`.
    if let Some(groups) = capability_groups.as_deref()
        && !groups.is_empty()
    {
        explicitly_scoped = true;
        tools.extend(get_tool_definitions_from_groups(ctx, groups));
    }
    if !explicitly_scoped && tools_enabled_all {
        tools.extend(get_tool_definitions_from_capabilities(ctx));
    }
    dedupe_tools_by_name(&mut tools);
    // Opt-in sub-agent fan-out: a coordinator with `enable_delegate` also gets
    // the synthetic `delegate` tool, letting it spawn focused specialist
    // sub-agents at runtime (the spawn/delegate AIS ops are compile-time only).
    if delegate_enabled(node) {
        tools.push(delegate_tool_definition());
    }
    tools
}

fn dedupe_tools_by_name(tools: &mut Vec<ToolDefinition>) {
    let mut seen = std::collections::HashSet::new();
    tools.retain(|tool| seen.insert(tool.name.clone()));
}

fn delegate_enabled(node: &Node) -> bool {
    // Accept either a JSON bool (`true`) or the AIR string attribute (`"true"`),
    // because converse-mode nodes may author either representation.
    node.attributes
        .get(graph_attrs::ENABLE_DELEGATE)
        .map(|v| v.as_bool() == Some(true) || v.as_str() == Some("true"))
        .unwrap_or(false)
}

fn delegate_tool_definition() -> ToolDefinition {
    ToolDefinition::new(
        DELEGATE_TOOL,
        "Delegate a focused subtask to a specialist sub-agent that runs with the \
 named tool groups and returns its findings. Issue several delegate calls \
 in one turn to investigate multiple areas in parallel.",
        serde_json::json!({
        "type": "object",
        "properties": {
        "task": {
        "type": "string",
        "description": "The focused subtask for the specialist to investigate and report on."
        },
        "capability_groups": {
        "type": "array",
        "items": {"type": "string"},
        "description": "Tool groups the sub-agent may use (e.g. a module's tool group)."
        }
        },
        "required": ["task"]
        }),
    )
}

pub(crate) fn inject_visible_skill_imports(
    tool_name: &str,
    args: &mut HashMap<String, Value>,
    metadata: &HashMap<String, String>,
) {
    if tool_name != capabilities::SEARCH_SKILLS || args.contains_key("imports") {
        return;
    }
    let Some(visible) = metadata.get(crate::metadata_keys::VISIBLE_SKILLS) else {
        return;
    };
    let imports: Vec<Value> = visible
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(|item| Value::String(item.to_string()))
        .collect();
    if !imports.is_empty() {
        args.insert("imports".to_string(), Value::Array(imports));
    }
}

/// Execute a single tool call.
///
/// Dispatch order:
/// 1. Python tool bridge (if attached and the tool name is registered).
/// 2. Rust capability system (built-ins and Rust-registered capabilities).
///
/// LLM-issued `tool_calls` carry only a name + JSON args, so the runtime
/// resolves them by name; the bridge's manifest (loaded from the artifact's
/// `python_tools` section) is the authoritative source for which names are
/// Python-backed.
/// Run a focused specialist sub-agent for a `delegate` tool call: a fresh
/// tool-using ASK over the requested tool groups, returning its findings as the
/// tool result. Boxed because it re-enters `execute_ask_with_tools` (mutual
/// recursion). Coordinators issue several `delegate`s in one turn; the parallel
/// dispatcher fans them out concurrently.
async fn execute_delegate(
    ctx: &ExecutionContext,
    parent_node: &Node,
    tool_call: &ToolCall,
    args: &HashMap<String, Value>,
) -> ToolResult {
    let task = args
        .get("task")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if task.is_empty() {
        return ToolResult::error(&tool_call.id, "delegate requires a non-empty 'task'");
    }

    // Synthetic specialist node: inherits the coordinator's backend/model
    // (and model_profile), scoped to the requested tool groups. It does NOT
    // carry `enable_delegate`, so specialists are leaf agents that cannot
    // recurse.
    let mut synth = Node::new(parent_node.id, AISOperationType::Ask);
    for attr in [
        graph_attrs::MODEL,
        graph_attrs::BACKEND,
        graph_attrs::MODEL_PROFILE,
    ] {
        if let Some(value) = parent_node.attributes.get(attr) {
            synth.attributes.insert(attr.to_string(), value.clone());
        }
    }
    if let Some(Value::Array(groups)) = args.get("capability_groups") {
        synth.attributes.insert(
            graph_attrs::CAPABILITY_GROUPS.to_string(),
            Value::Array(groups.clone()),
        );
    }

    let persona = "You are a focused specialist sub-agent. Investigate ONLY the \
 delegated task using your tools, then report concise, factual findings with \
 no preamble. If a needed tool is unavailable, say so plainly.";
    let req = match apply_llm_request_routing_from_node(
        LLMRequest::new(task).with_system_prompt(persona),
        &synth,
    ) {
        Ok(req) => req,
        Err(e) => {
            return ToolResult::error(&tool_call.id, format!("delegate routing failed: {e}"));
        }
    };
    let tools = resolve_ask_tools(ctx, &synth);
    let req = if tools.is_empty() {
        req
    } else {
        attach_graph_hints(ctx, &synth, req)
            .with_tools(tools)
            .with_tool_choice(ToolChoice::Auto)
    };

    match Box::pin(execute_ask_with_tools(ctx, &synth, &req)).await {
        Ok(Value::String(s)) => ToolResult::success(&tool_call.id, s),
        Ok(other) => ToolResult::success(&tool_call.id, format!("{other:?}")),
        Err(e) => ToolResult::error(&tool_call.id, format!("delegate sub-agent failed: {e}")),
    }
}

async fn execute_tool_call(
    ctx: &ExecutionContext,
    node: &Node,
    tool_call: &ToolCall,
) -> ToolResult {
    apxm_llm!(debug,
     execution_id = %ctx.execution_id,
     tool_name = %tool_call.name,
     tool_id = %tool_call.id,
     "Executing tool call"
    );

    let mut args: HashMap<String, Value> = match &tool_call.args {
        serde_json::Value::Object(obj) => obj
            .iter()
            .map(|(k, v)| (k.clone(), json_to_value(v)))
            .collect(),
        _ => HashMap::new(),
    };
    inject_visible_skill_imports(&tool_call.name, &mut args, &ctx.metadata);

    if let Some(emitter) = &ctx.event_emitter {
        emitter.emit_tool_start(&tool_call.name, &args);
    }

    // Native sub-agent fan-out: `delegate` is not a capability — it re-enters
    // the ASK tool loop as a focused specialist over the requested tool groups.
    if tool_call.name == DELEGATE_TOOL {
        let result = execute_delegate(ctx, node, tool_call, &args).await;
        if let Some(emitter) = &ctx.event_emitter {
            emitter.emit_tool_end(&tool_call.name, &Value::String(result.content.clone()));
        }
        return result;
    }

    if let Some(bridge) = ctx.python_handler_bridge.as_ref() {
        if bridge.has_tool(&tool_call.name) {
            return dispatch_script_tool_call(ctx, node, tool_call, args, "Python").await;
        }
    }
    if let Some(bridge) = ctx.typescript_handler_bridge.as_ref() {
        if bridge.has_tool(&tool_call.name) {
            return dispatch_script_tool_call(ctx, node, tool_call, args, "TypeScript").await;
        }
    }

    // Native/builtin tool path also runs pre/post_cap hooks. A pre_cap deny continues the turn gracefully (m4).
    let args =
        match crate::executor::hook_driver::run_pre_cap_hooks(ctx, &tool_call.name, args).await {
            Ok(edited) => edited,
            Err(e) => {
                if let Some(emitter) = &ctx.event_emitter {
                    emitter.emit_tool_end(&tool_call.name, &Value::String(e.to_string()));
                }
                return ToolResult::error(&tool_call.id, e.to_string());
            }
        };
    if let Err(error) = super::super::inv_cap::enforce_write_boundary(
        ctx,
        &tool_call.name,
        &args,
        false,
        &tool_call.id,
    )
    .await
    {
        if let Some(emitter) = &ctx.event_emitter {
            emitter.emit_tool_end(&tool_call.name, &Value::String(error.to_string()));
        }
        return ToolResult::error(&tool_call.id, error.to_string());
    }
    let timeout =
        std::time::Duration::from_millis(apxm_core::constants::defaults::DEFAULT_TIMEOUT_MS);
    match ctx
        .invoke_capability_with_timeout_for_call(&tool_call.name, args, timeout, &tool_call.id)
        .await
    {
        Ok(result) => {
            let result =
                crate::executor::hook_driver::run_post_cap_hooks(ctx, &tool_call.name, result)
                    .await;
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
// Per-capability write serialization is shared with the graph/inv_cap path via
// `crate::capability::tool_write_lock` so same-name writes serialize on BOTH
// parallelism engines (not just this in-ASK loop).
use crate::capability::tool_write_lock::{release_write_lock_if_idle, write_lock_for_tool};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToolAccess {
    ReadOnly,
    Write,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ScriptToolPolicy {
    pub read_only: bool,
    pub requires_approval: bool,
}

fn checked_script_tool_policy(
    name: &str,
    read_only: Option<bool>,
    requires_approval: Option<bool>,
) -> std::result::Result<ScriptToolPolicy, RuntimeError> {
    let (Some(read_only), Some(requires_approval)) = (read_only, requires_approval) else {
        return Err(RuntimeError::Capability {
            capability: name.to_string(),
            message: format!(
                "script capability '{name}' is missing joined read_only/requires_approval policy \
                 metadata; rebuild the agent package"
            ),
        });
    };
    Ok(ScriptToolPolicy {
        read_only,
        requires_approval,
    })
}

pub(crate) fn script_tool_policy(
    ctx: &ExecutionContext,
    name: &str,
) -> std::result::Result<ScriptToolPolicy, RuntimeError> {
    if let Some(descriptor) = ctx
        .python_handler_bridge
        .as_ref()
        .and_then(|bridge| bridge.registry().resolve(name))
    {
        return checked_script_tool_policy(
            name,
            descriptor.read_only,
            descriptor.requires_approval,
        );
    }
    if let Some(descriptor) = ctx
        .typescript_handler_bridge
        .as_ref()
        .and_then(|bridge| bridge.registry().resolve(name))
    {
        return checked_script_tool_policy(
            name,
            descriptor.read_only,
            descriptor.requires_approval,
        );
    }
    Err(RuntimeError::Capability {
        capability: name.to_string(),
        message: format!("script capability '{name}' is not registered in an artifact worker"),
    })
}

fn resolve_tool_access(ctx: &ExecutionContext, name: &str) -> Option<ToolAccess> {
    if let Some(metadata) = ctx.capability_system.get_metadata(name) {
        return Some(if metadata.read_only {
            ToolAccess::ReadOnly
        } else {
            ToolAccess::Write
        });
    }

    if ctx
        .python_handler_bridge
        .as_ref()
        .is_some_and(|bridge| bridge.has_tool(name))
    {
        return Some(match script_tool_policy(ctx, name) {
            Ok(policy) if policy.read_only => ToolAccess::ReadOnly,
            _ => ToolAccess::Write,
        });
    }

    if ctx
        .typescript_handler_bridge
        .as_ref()
        .is_some_and(|bridge| bridge.has_tool(name))
    {
        return Some(match script_tool_policy(ctx, name) {
            Ok(policy) if policy.read_only => ToolAccess::ReadOnly,
            _ => ToolAccess::Write,
        });
    }

    None
}

async fn dispatch_script_tool_call(
    ctx: &ExecutionContext,
    _node: &Node,
    tool_call: &ToolCall,
    args: HashMap<String, Value>,
    label: &str,
) -> ToolResult {
    let timeout =
        std::time::Duration::from_millis(apxm_core::constants::defaults::DEFAULT_TIMEOUT_MS);
    let edited_args =
        match crate::executor::hook_driver::run_pre_cap_hooks(ctx, &tool_call.name, args).await {
            Ok(a) => a,
            Err(e) => {
                if let Some(emitter) = &ctx.event_emitter {
                    emitter.emit_tool_end(&tool_call.name, &Value::String(e.to_string()));
                }
                return ToolResult::error(&tool_call.id, e.to_string());
            }
        };
    let policy = match script_tool_policy(ctx, &tool_call.name) {
        Ok(policy) => policy,
        Err(error) => {
            if let Some(emitter) = &ctx.event_emitter {
                emitter.emit_tool_end(&tool_call.name, &Value::String(error.to_string()));
            }
            return ToolResult::error(&tool_call.id, error.to_string());
        }
    };
    if let Err(error) = super::super::inv_cap::enforce_write_boundary(
        ctx,
        &tool_call.name,
        &edited_args,
        !policy.read_only,
        &tool_call.id,
    )
    .await
    {
        if let Some(emitter) = &ctx.event_emitter {
            emitter.emit_tool_end(&tool_call.name, &Value::String(error.to_string()));
        }
        return ToolResult::error(&tool_call.id, error.to_string());
    }
    let admitted_args = match ctx
        .admit_capability_call(
            &tool_call.name,
            edited_args,
            policy.requires_approval,
            &tool_call.id,
        )
        .await
    {
        Ok(args) => args,
        Err(error) => {
            if let Some(emitter) = &ctx.event_emitter {
                emitter.emit_tool_end(&tool_call.name, &Value::String(error.to_string()));
            }
            return ToolResult::error(&tool_call.id, error.to_string());
        }
    };
    let json_args = serde_json::to_value(&admitted_args).unwrap_or_else(|_| tool_call.args.clone());
    let bridge_call = async {
        if let Some(bridge) = ctx.python_handler_bridge.as_ref() {
            if bridge.has_tool(&tool_call.name) {
                return bridge
                    .call(&tool_call.name, json_args.clone(), timeout)
                    .await;
            }
        }
        let bridge = ctx.typescript_handler_bridge.as_ref().ok_or_else(|| {
            apxm_core::error::RuntimeError::Capability {
                capability: tool_call.name.clone(),
                message: format!("no {label} handler bridge configured"),
            }
        })?;
        bridge.call(&tool_call.name, json_args, timeout).await
    };

    match bridge_call.await {
        Ok(json_result) => {
            let raw = Value::try_from(json_result).unwrap_or(Value::Null);
            let transformed =
                crate::executor::hook_driver::run_post_cap_hooks(ctx, &tool_call.name, raw).await;
            let content = match transformed {
                Value::String(s) => s,
                other => other.to_string(),
            };
            if let Some(emitter) = &ctx.event_emitter {
                emitter.emit_tool_end(&tool_call.name, &Value::String(content.clone()));
            }
            apxm_llm!(info,
             execution_id = %ctx.execution_id,
             tool_name = %tool_call.name,
             bridge = %label,
             "Script tool call succeeded"
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
             bridge = %label,
             error = %e,
             "Script tool call failed"
            );
            ToolResult::error(&tool_call.id, e.to_string())
        }
    }
}

/// Execute multiple tool calls concurrently, preserving result order.
///
/// Read-only tools (according to `RuntimeCapability::read_only`) run in full
/// parallel. Write tools acquire a per-tool-name `RwLock` so that:
/// - Multiple read-only calls execute simultaneously.
/// - Write calls on the same tool name are serialized.
/// - Write calls on different tool names run in parallel.
async fn execute_tool_calls_parallel(
    ctx: &ExecutionContext,
    node: &Node,
    tool_calls: &[ToolCall],
) -> Vec<ToolResult> {
    if tool_calls.len() == 1 {
        return vec![execute_tool_call(ctx, node, &tool_calls[0]).await];
    }

    let max_parallel = max_parallel_tool_calls(ctx, tool_calls.len());
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
        results.extend(execute_tool_call_batch(ctx, node, batch).await);
    }
    results
}

async fn execute_tool_call_batch(
    ctx: &ExecutionContext,
    node: &Node,
    tool_calls: &[ToolCall],
) -> Vec<ToolResult> {
    let futures: Vec<_> = tool_calls
        .iter()
        .map(|tc| {
            let access = resolve_tool_access(ctx, &tc.name);
            async move {
                match access {
                    Some(ToolAccess::ReadOnly) | None => execute_tool_call(ctx, node, tc).await,
                    Some(ToolAccess::Write) => {
                        let lock = write_lock_for_tool(&tc.name);
                        let result = {
                            let _guard = lock.write().await;
                            execute_tool_call(ctx, node, tc).await
                        };
                        release_write_lock_if_idle(&tc.name, &lock);
                        result
                    }
                }
            }
        })
        .collect();

    futures::future::join_all(futures).await
}

fn max_parallel_tool_calls(ctx: &ExecutionContext, tool_count: usize) -> usize {
    clamp_tool_call_parallelism(tool_count, ctx.max_parallel_tool_calls)
}

fn clamp_tool_call_parallelism(tool_count: usize, configured: usize) -> usize {
    if tool_count == 0 {
        0
    } else {
        configured
            .clamp(
                1,
                crate::LlmToolDispatchConfig::HARD_MAX_PARALLEL_TOOL_CALLS,
            )
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
pub(crate) async fn execute_ask_with_tools(
    ctx: &ExecutionContext,
    node: &Node,
    initial_request: &LLMRequest,
) -> Result<Value> {
    let output = execute_ask_with_tools_attempt(ctx, node, initial_request).await?;
    emit_llm_done(ctx, &output.final_response);
    Ok(output.value)
}

pub(super) async fn execute_ask_with_tools_attempt(
    ctx: &ExecutionContext,
    node: &Node,
    initial_request: &LLMRequest,
) -> Result<LlmAttemptOutput> {
    let max_iterations = node
        .attributes
        .get(graph_attrs::MAX_TOOL_ITERATIONS)
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
        .unwrap_or(DEFAULT_MAX_TOOL_ITERATIONS);

    let mut current_request = initial_request.clone();
    // Only the running count of invoked tools is ever read (for logging below);
    // keep a counter rather than accumulating and cloning the results each round.
    let mut tools_invoked_count: usize = 0;
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
            // also feed the process-wide meter so `/v1/generate`
            // (which bypasses this executor path entirely) and the executor
            // path are observed through one shared counter.
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

        super::emit_model_step(
            ctx,
            node.id,
            iteration + 1,
            &response,
            iter_total_ms,
            iter_prefill,
            iter_decode,
        );

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
             tools_invoked = tools_invoked_count,
             "ASK tool loop completed"
            );
            ctx.timing_tracker
                .record(node.id, total_prefill_ms, total_decode_ms);
            return Ok(LlmAttemptOutput {
                value: Value::String(response.content.clone()),
                final_response: response,
            });
        }

        let tool_results = execute_tool_calls_parallel(ctx, node, &response.tool_calls).await;

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

        tools_invoked_count += tool_results.len();

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
            "Tool loop exceeded maximum iterations ({}). {} tool calls executed.",
            max_iterations, tools_invoked_count
        ),
        backend: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aam::Aam;
    use crate::capability::CapabilitySystem;
    use crate::capability::executor::CapabilityExecutor;
    use crate::capability::interceptor::{CapabilityInterceptor, InterceptDecision};
    use crate::capability::metadata::RuntimeCapability;
    use crate::memory::{MemoryConfig, MemorySystem};
    use apxm_backends::LLMRegistry;
    use apxm_core::types::consent::{
        ApprovalEvidence, ConsentBroker, ConsentDecision, InteractiveApproval, PermissionPrompt,
    };
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct RecordingCapability {
        metadata: RuntimeCapability,
        calls: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl CapabilityExecutor for RecordingCapability {
        async fn execute(
            &self,
            _args: HashMap<String, Value>,
        ) -> std::result::Result<Value, apxm_core::error::RuntimeError> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            Ok(Value::String("executed".to_string()))
        }

        fn metadata(&self) -> &RuntimeCapability {
            &self.metadata
        }
    }

    struct StubBroker {
        decision: ConsentDecision,
        calls: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl ConsentBroker for StubBroker {
        async fn request_consent(
            &self,
            _prompt: PermissionPrompt,
            _timeout: std::time::Duration,
        ) -> ConsentDecision {
            self.calls.fetch_add(1, Ordering::Relaxed);
            self.decision.clone()
        }
    }

    struct CountingInterceptor {
        calls: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl CapabilityInterceptor for CountingInterceptor {
        fn name(&self) -> &str {
            "counting-test"
        }

        async fn pre_invoke(
            &self,
            _name: &str,
            _args: &HashMap<String, Value>,
        ) -> InterceptDecision {
            self.calls.fetch_add(1, Ordering::Relaxed);
            InterceptDecision::Allow
        }
    }

    async fn model_tool_context(
        name: &str,
        read_only: bool,
        requires_approval: bool,
    ) -> (ExecutionContext, Arc<CapabilitySystem>, Arc<AtomicUsize>) {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .expect("memory"),
        );
        let aam = Aam::new();
        let capability_system = Arc::new(CapabilitySystem::with_aam(aam.clone()));
        let calls = Arc::new(AtomicUsize::new(0));
        let mut metadata = RuntimeCapability::new(
            name,
            "record model tool admission",
            serde_json::json!({"type": "object"}),
        );
        if read_only {
            metadata = metadata.with_read_only();
        }
        if requires_approval {
            metadata = metadata.with_requires_approval();
        }
        capability_system
            .register(Arc::new(RecordingCapability {
                metadata,
                calls: Arc::clone(&calls),
            }))
            .expect("recording capability");
        let facade: Arc<dyn apxm_capability_iface::CapabilityFacade> = capability_system.clone();
        let ctx = ExecutionContext::new(memory, Arc::new(LLMRegistry::new()), facade, aam);
        (ctx, capability_system, calls)
    }

    fn approved() -> ConsentDecision {
        ConsentDecision::Approved {
            evidence: ApprovalEvidence::Interactive(InteractiveApproval {
                decided_at: "2026-07-11T00:00:00Z".into(),
                responder_subject: Some("operator".into()),
            }),
        }
    }

    fn visible_metadata(value: &str) -> HashMap<String, String> {
        HashMap::from([(
            crate::metadata_keys::VISIBLE_SKILLS.to_string(),
            value.to_string(),
        )])
    }

    #[test]
    fn search_skills_inherits_execution_visible_imports() {
        let mut args = HashMap::from([("request".to_string(), Value::String("review".into()))]);

        inject_visible_skill_imports(
            capabilities::SEARCH_SKILLS,
            &mut args,
            &visible_metadata("support, engineering,,docs "),
        );

        assert_eq!(
            args.get("imports"),
            Some(&Value::Array(vec![
                Value::String("support".into()),
                Value::String("engineering".into()),
                Value::String("docs".into()),
            ]))
        );
    }

    #[test]
    fn explicit_search_skills_imports_are_preserved() {
        let mut args = HashMap::from([(
            "imports".to_string(),
            Value::Array(vec![Value::String("security".into())]),
        )]);

        inject_visible_skill_imports(
            capabilities::SEARCH_SKILLS,
            &mut args,
            &visible_metadata("support,engineering"),
        );

        assert_eq!(
            args.get("imports"),
            Some(&Value::Array(vec![Value::String("security".into())]))
        );
    }

    #[test]
    fn non_discovery_tools_do_not_receive_skill_imports() {
        let mut args = HashMap::new();

        inject_visible_skill_imports("http_get", &mut args, &visible_metadata("support"));

        assert!(!args.contains_key("imports"));
    }

    #[tokio::test]
    async fn model_tool_call_requires_consent_and_runs_interceptors() {
        let (mut ctx, capability_system, capability_calls) =
            model_tool_context("approval-tool", true, true).await;
        let interceptor_calls = Arc::new(AtomicUsize::new(0));
        capability_system.register_interceptor(Arc::new(CountingInterceptor {
            calls: Arc::clone(&interceptor_calls),
        }));
        let node = Node::new(1, AISOperationType::Ask);
        let tool_call = ToolCall::new("call-1", "approval-tool", serde_json::json!({}));

        let without_broker = execute_tool_call(&ctx, &node, &tool_call).await;

        assert!(!without_broker.success);
        assert!(
            without_broker
                .content
                .contains(apxm_core::types::consent::APPROVAL_BROKER_UNAVAILABLE_REASON)
        );
        assert_eq!(capability_calls.load(Ordering::Relaxed), 0);
        assert_eq!(interceptor_calls.load(Ordering::Relaxed), 0);

        let broker_calls = Arc::new(AtomicUsize::new(0));
        ctx.consent_broker = Arc::new(StubBroker {
            decision: approved(),
            calls: Arc::clone(&broker_calls),
        });

        let approved_result = execute_tool_call(&ctx, &node, &tool_call).await;

        assert!(approved_result.success, "{}", approved_result.content);
        assert_eq!(approved_result.content, "executed");
        assert_eq!(broker_calls.load(Ordering::Relaxed), 1);
        assert_eq!(interceptor_calls.load(Ordering::Relaxed), 1);
        assert_eq!(capability_calls.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn open_read_only_model_tool_skips_consent_but_runs_interceptors() {
        let (mut ctx, capability_system, capability_calls) =
            model_tool_context("read-only-tool", true, false).await;
        let broker_calls = Arc::new(AtomicUsize::new(0));
        ctx.consent_broker = Arc::new(StubBroker {
            decision: ConsentDecision::Denied {
                reason: "broker must not be called".into(),
            },
            calls: Arc::clone(&broker_calls),
        });
        let interceptor_calls = Arc::new(AtomicUsize::new(0));
        capability_system.register_interceptor(Arc::new(CountingInterceptor {
            calls: Arc::clone(&interceptor_calls),
        }));
        let node = Node::new(1, AISOperationType::Ask);
        let tool_call = ToolCall::new("call-1", "read-only-tool", serde_json::json!({}));

        let result = execute_tool_call(&ctx, &node, &tool_call).await;

        assert!(result.success, "{}", result.content);
        assert_eq!(result.content, "executed");
        assert_eq!(broker_calls.load(Ordering::Relaxed), 0);
        assert_eq!(interceptor_calls.load(Ordering::Relaxed), 1);
        assert_eq!(capability_calls.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn model_write_tool_without_grant_is_denied_before_execution() {
        let (ctx, _capability_system, capability_calls) =
            model_tool_context("write-tool", false, false).await;
        let node = Node::new(1, AISOperationType::Ask);
        let tool_call = ToolCall::new("call-1", "write-tool", serde_json::json!({}));

        let result = execute_tool_call(&ctx, &node, &tool_call).await;

        assert!(!result.success);
        assert!(result.content.contains("missing a capability grant"));
        assert_eq!(capability_calls.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn script_tool_policy_requires_joined_metadata() {
        let error = checked_script_tool_policy("script-tool", None, Some(false))
            .expect_err("missing read_only metadata must fail closed");

        assert!(matches!(
            error,
            RuntimeError::Capability { ref capability, ref message }
                if capability == "script-tool" && message.contains("missing joined")
        ));
    }

    #[test]
    fn script_tool_policy_preserves_open_read_only_metadata() {
        assert_eq!(
            checked_script_tool_policy("script-tool", Some(true), Some(false))
                .expect("complete policy metadata"),
            ScriptToolPolicy {
                read_only: true,
                requires_approval: false,
            }
        );
    }
}
