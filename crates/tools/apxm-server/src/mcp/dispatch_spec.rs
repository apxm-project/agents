//! Tier-2 dynamic dispatch: the agent emits a CONSTRAINED spec describing a
//! sub-agent fan-out, the server validates it (breadth cap + dup/dangling/cycle
//! checks), templates it into AIR, and runs it via the shared `run_air_inner`
//! core (so the invoke-site write boundary + admission apply). This is the
//! safer default than raw AIR (`apxm_run`): the agent decides the *topology*
//! and prompts, but the lowering is a fixed, auditable template — matching the
//! "deterministic topology, model-driven decision, pre-vetted leaves" best
//! practice.

use axum::Json;
use serde::Deserialize;
use serde_json::Value as JsonValue;

use crate::execute::{ExecuteRequest, run_air_inner};
use crate::helpers::mcp_tool_result;
use crate::state::AppState;

pub(crate) const MCP_TOOL_APXM_DISPATCH: &str = "apxm_dispatch";

/// Fork-bomb breadth cap: the maximum number of sub-agents a single dispatch
/// may fan out to. Bounds the per-request graph width (depth is bounded by the
/// invoke-site/CALL_SKILL limits independently).
const MAX_SUB_AGENTS: usize = 16;

#[derive(Debug, Deserialize)]
struct DispatchSpec {
    /// The graph steps. `sub_agents` is accepted as a back-compat alias.
    #[serde(alias = "sub_agents")]
    steps: Vec<StepSpec>,
    #[serde(default)]
    admit_capabilities: Vec<String>,
}

/// One node of the dispatched graph: an LLM sub-agent (`kind: "agent"`, the
/// default) or a direct tool call (`kind: "tool"`). Independent steps (no
/// `depends_on`) run in PARALLEL via the dataflow scheduler — this is how the
/// agent composes "a workflow of multiple parallel tools".
#[derive(Debug, Deserialize)]
struct StepSpec {
    /// Stable id used to wire `depends_on` and `{id}` references.
    id: String,
    /// "agent" (ASK) or "tool" (inv_tool). Defaults to "agent".
    #[serde(default = "default_kind")]
    kind: String,
    /// Role / persona prefix for an agent step's prompt.
    #[serde(default)]
    role: String,
    /// Agent prompt (required for kind="agent"); may reference upstream `{id}`.
    #[serde(default)]
    prompt: String,
    /// Capability name (required for kind="tool").
    #[serde(default)]
    capability: String,
    /// Tool params object (for kind="tool"); serialized to the inv_tool params_json.
    #[serde(default)]
    params: JsonValue,
    /// Ids of steps whose outputs feed this one (data dependencies).
    #[serde(default)]
    depends_on: Vec<String>,
}

fn default_kind() -> String {
    "agent".to_string()
}

pub(crate) fn dispatch_input_schema() -> JsonValue {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["steps"],
        "properties": {
            "steps": {
                "type": "array",
                "description": "graph nodes; independent steps (no depends_on) run in parallel",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["id"],
                    "properties": {
                        "id": { "type": "string" },
                        "kind": { "type": "string", "enum": ["agent", "tool"], "description": "agent=ASK (needs prompt), tool=inv_tool (needs capability). default agent" },
                        "role": { "type": "string" },
                        "prompt": { "type": "string", "description": "for kind=agent" },
                        "capability": { "type": "string", "description": "for kind=tool" },
                        "params": { "type": "object", "description": "for kind=tool" },
                        "depends_on": { "type": "array", "items": { "type": "string" } }
                    }
                }
            },
            "admit_capabilities": { "type": "array", "items": { "type": "string" } }
        }
    })
}

/// Async MCP handler for `apxm_dispatch`. Returns `Some` if it owns `tool_name`.
pub(crate) async fn call_dispatch_tool(
    state: &AppState,
    id: &JsonValue,
    tool_name: &str,
    args: &JsonValue,
) -> Option<Json<JsonValue>> {
    if tool_name != MCP_TOOL_APXM_DISPATCH {
        return None;
    }
    let spec: DispatchSpec = match serde_json::from_value(args.clone()) {
        Ok(spec) => spec,
        Err(error) => {
            return Some(mcp_tool_result(
                id.clone(),
                format!("invalid apxm_dispatch spec: {error}"),
                true,
            ));
        }
    };
    let air = match spec_to_air(&spec) {
        Ok(air) => air,
        Err(error) => return Some(mcp_tool_result(id.clone(), error, true)),
    };
    let req = ExecuteRequest {
        air,
        args: Vec::new(),
        session_id: None,
        session_root: None,
        admit_capabilities: spec.admit_capabilities.clone(),
        imports: Vec::new(),
    };
    match run_air_inner(state, req).await {
        Ok(response) => {
            let text = serde_json::to_string(&response)
                .unwrap_or_else(|error| format!("{{\"error\":\"serialize failed: {error}\"}}"));
            Some(mcp_tool_result(id.clone(), text, false))
        }
        Err(error) => Some(mcp_tool_result(id.clone(), error.message, true)),
    }
}

/// Validate the spec and template it into a fan-out AIR graph. Each step is an
/// `ais.ask` (agent) or `ais.inv_tool` (tool) whose inputs are its `depends_on`
/// outputs; independent steps run in PARALLEL on the scheduler; a final
/// `ais.merge` joins every step and is returned.
fn spec_to_air(spec: &DispatchSpec) -> Result<String, String> {
    if spec.steps.is_empty() {
        return Err("apxm_dispatch: steps must be non-empty".to_string());
    }
    if spec.steps.len() > MAX_SUB_AGENTS {
        return Err(format!(
            "apxm_dispatch: {} steps exceeds the breadth cap of {MAX_SUB_AGENTS}",
            spec.steps.len()
        ));
    }
    // Unique ids.
    let mut seen = std::collections::HashSet::new();
    for st in &spec.steps {
        if st.id.is_empty() {
            return Err("apxm_dispatch: every step needs a non-empty id".to_string());
        }
        if !seen.insert(st.id.as_str()) {
            return Err(format!("apxm_dispatch: duplicate step id '{}'", st.id));
        }
    }
    // depends_on must reference earlier ids (topological order requirement also
    // rejects cycles and forward refs in one pass).
    let mut defined: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut body = String::new();
    let mut ssa_of: std::collections::HashMap<&str, String> = std::collections::HashMap::new();
    for (i, st) in spec.steps.iter().enumerate() {
        for dep in &st.depends_on {
            if !defined.contains(dep.as_str()) {
                return Err(format!(
                    "apxm_dispatch: step '{}' depends_on '{dep}' which is not a prior id \
                     (forward reference or cycle)",
                    st.id
                ));
            }
        }
        let ssa = format!("%n{i}");
        let line = match st.kind.as_str() {
            "tool" => {
                if st.capability.is_empty() {
                    return Err(format!(
                        "apxm_dispatch: tool step '{}' requires a 'capability'",
                        st.id
                    ));
                }
                let params = if st.params.is_null() {
                    "{}".to_string()
                } else {
                    st.params.to_string()
                };
                let mut l = format!(
                    "    {ssa} = ais.inv_tool {} ({})",
                    quote_air(&st.capability),
                    quote_air(&params)
                );
                if !st.depends_on.is_empty() {
                    let (refs, types, names) = dep_parts(st, &ssa_of);
                    l.push_str(&format!(
                        " [{} : {}] {{input_names = [{}]}}",
                        refs, types, names
                    ));
                }
                l.push_str(" : !ais.token\n");
                l
            }
            _ => {
                if st.prompt.is_empty() {
                    return Err(format!(
                        "apxm_dispatch: agent step '{}' requires a 'prompt'",
                        st.id
                    ));
                }
                let prompt = if st.role.is_empty() {
                    st.prompt.clone()
                } else {
                    format!("{}: {}", st.role, st.prompt)
                };
                let mut l = format!("    {ssa} = ais.ask {}", quote_air(&prompt));
                if !st.depends_on.is_empty() {
                    let (refs, types, names) = dep_parts(st, &ssa_of);
                    l.push_str(&format!(
                        " [{} : {}] {{input_names = [{}]}}",
                        refs, types, names
                    ));
                }
                l.push_str(" : !ais.token\n");
                l
            }
        };
        body.push_str(&line);
        ssa_of.insert(st.id.as_str(), ssa);
        defined.insert(st.id.as_str());
    }
    // Merge every sub-agent output into the returned token.
    let all: Vec<String> = (0..spec.steps.len()).map(|i| format!("%n{i}")).collect();
    let types = vec!["!ais.token"; all.len()].join(", ");
    body.push_str(&format!(
        "    %ret = ais.merge {} : {} -> !ais.token\n    func.return %ret : !ais.token\n",
        all.join(", "),
        types
    ));
    Ok(format!(
        "module {{\n  func.func @apxm_dispatch() -> !ais.token attributes {{ais.entry}} {{\n{body}  }}\n}}\n"
    ))
}

/// Build the `(ssa_refs, types, input_names)` strings for a step's `depends_on`.
fn dep_parts(
    st: &StepSpec,
    ssa_of: &std::collections::HashMap<&str, String>,
) -> (String, String, String) {
    let refs: Vec<String> = st
        .depends_on
        .iter()
        .map(|d| ssa_of[d.as_str()].to_string())
        .collect();
    let types = vec!["!ais.token"; refs.len()].join(", ");
    let names: Vec<String> = st.depends_on.iter().map(|d| quote_air(d)).collect();
    (refs.join(", "), types, names.join(", "))
}

/// Quote a string as an MLIR string literal.
fn quote_air(s: &str) -> String {
    let escaped = s
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\t', "\\t")
        .replace('\r', "\\r");
    format!("\"{escaped}\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(json: JsonValue) -> DispatchSpec {
        serde_json::from_value(json).unwrap()
    }

    #[test]
    fn templates_fanout_with_dependencies() {
        let s = spec(serde_json::json!({"sub_agents": [
            {"id": "r1", "role": "researcher", "prompt": "research A"},
            {"id": "r2", "prompt": "research B"},
            {"id": "syn", "prompt": "combine {r1} {r2}", "depends_on": ["r1", "r2"]}
        ]}));
        let air = spec_to_air(&s).expect("valid");
        assert!(air.contains("ais.ask \"researcher: research A\""));
        assert!(air.contains("input_names = [\"r1\", \"r2\"]"));
        assert!(air.contains("ais.merge %n0, %n1, %n2"));
        assert!(air.contains("ais.entry"));
    }

    #[test]
    fn rejects_breadth_over_cap() {
        let agents: Vec<JsonValue> = (0..(MAX_SUB_AGENTS + 1))
            .map(|i| serde_json::json!({"id": format!("a{i}"), "prompt": "x"}))
            .collect();
        let s = spec(serde_json::json!({"sub_agents": agents}));
        assert!(spec_to_air(&s).unwrap_err().contains("breadth cap"));
    }

    #[test]
    fn rejects_forward_ref_and_cycle() {
        let s = spec(serde_json::json!({"sub_agents": [
            {"id": "a", "prompt": "x", "depends_on": ["b"]},
            {"id": "b", "prompt": "y"}
        ]}));
        assert!(
            spec_to_air(&s)
                .unwrap_err()
                .contains("forward reference or cycle")
        );
    }

    #[test]
    fn rejects_duplicate_ids() {
        let s = spec(serde_json::json!({"steps": [
            {"id": "a", "prompt": "x"}, {"id": "a", "prompt": "y"}
        ]}));
        assert!(spec_to_air(&s).unwrap_err().contains("duplicate"));
    }

    #[test]
    fn templates_tool_steps_and_mixed_graph() {
        // A graph of two parallel tools + an agent that depends on both.
        let s = spec(serde_json::json!({"steps": [
            {"id": "f1", "kind": "tool", "capability": "read", "params": {"path": "/a"}},
            {"id": "f2", "kind": "tool", "capability": "read", "params": {"path": "/b"}},
            {"id": "sum", "kind": "agent", "prompt": "merge {f1} {f2}", "depends_on": ["f1", "f2"]}
        ]}));
        let air = spec_to_air(&s).expect("valid mixed graph");
        assert!(air.contains("ais.inv_tool \"read\""));
        assert!(air.contains("ais.ask \"merge"));
        assert!(air.contains("ais.merge %n0, %n1, %n2"));
    }

    #[test]
    fn tool_step_requires_capability() {
        let s = spec(serde_json::json!({"steps": [{"id": "t", "kind": "tool"}]}));
        assert!(
            spec_to_air(&s)
                .unwrap_err()
                .contains("requires a 'capability'")
        );
    }

    #[test]
    fn sub_agents_alias_still_accepted() {
        let s = spec(serde_json::json!({"sub_agents": [{"id": "a", "prompt": "x"}]}));
        assert!(spec_to_air(&s).is_ok());
    }
}
