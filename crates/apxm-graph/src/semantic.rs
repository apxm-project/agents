//! Semantic validation for APXM graphs (Tier 1 + Tier 2).
//!
//! Tier 1 (static): Runs with graph JSON alone. Catches placeholder bounds
//! errors, COMMUNICATE/SPAWN consistency, duplicate agent names, and
//! parameter arity mismatches.
//!
//! Tier 2 (environment): When a [`SemanticContext`] is populated, validates
//! that fields annotated with [`ReferenceType`] reference registered
//! resources (profiles, backends, models, capabilities).

use crate::{ApxmGraph, GraphNode};
use apxm_core::constants::{communicate_protocols, graph::attrs as graph_attrs};
use apxm_core::error::{Error, ErrorCode, Suggestion};
use apxm_core::types::identifiers::{BackendId, CapabilityName, ModelId, ProfileId};
use apxm_core::types::{AISOperationType, DependencyType, ReferenceType, get_operation_spec};
use std::collections::{HashMap, HashSet};

/// External resource registries for Tier 2 validation.
///
/// When empty (`default()`), only Tier 1 (pure graph) checks run.
#[derive(Default)]
pub struct SemanticContext {
    pub profiles: HashSet<ProfileId>,
    pub backends: HashSet<BackendId>,
    pub models: HashSet<ModelId>,
    pub capabilities: HashSet<CapabilityName>,
}

impl SemanticContext {
    pub fn with_capacity(
        profiles: usize,
        backends: usize,
        models: usize,
        capabilities: usize,
    ) -> Self {
        Self {
            profiles: HashSet::with_capacity(profiles),
            backends: HashSet::with_capacity(backends),
            models: HashSet::with_capacity(models),
            capabilities: HashSet::with_capacity(capabilities),
        }
    }

    fn contains(&self, ref_type: ReferenceType, name: &str) -> bool {
        match ref_type {
            ReferenceType::Profile => self.profiles.contains(name),
            ReferenceType::Backend => self.backends.contains(name),
            ReferenceType::Model => self.models.contains(name),
            ReferenceType::Capability => self.capabilities.contains(name),
        }
    }

    fn has_data(&self) -> bool {
        !self.profiles.is_empty()
            || !self.backends.is_empty()
            || !self.models.is_empty()
            || !self.capabilities.is_empty()
    }
}

/// Run semantic validation on the graph. Returns a list of errors/warnings.
///
/// Tier 1 checks always run. Tier 2 checks run only when `ctx` has data.
pub fn validate_semantic(graph: &ApxmGraph, ctx: &SemanticContext) -> Vec<Error> {
    let mut errors = Vec::new();
    let has_ctx = ctx.has_data();
    let incoming_data = count_incoming_data_edges(graph);

    // Pre-pass: collect spawned agent names for COMMUNICATE check (E502, E503)
    let mut spawned_agents: HashMap<&str, u64> = HashMap::new();
    for node in &graph.nodes {
        if node.op == AISOperationType::SpawnAgent {
            if let Some(name) = node
                .attributes
                .get(graph_attrs::AGENT_NAME)
                .and_then(|v| v.as_str())
            {
                if let Some(_prev_id) = spawned_agents.insert(name, node.id) {
                    errors.push(Error::new_global(
                        ErrorCode::DuplicateAgentName,
                        format!(
                            "node '{}': duplicate agent name '{}' in SPAWN_AGENT",
                            node.name, name
                        ),
                        &node.name,
                    ));
                }
            }
        }
    }

    // Main pass: op-specific + generic ref_type checks
    for node in &graph.nodes {
        let spec = get_operation_spec(node.op);
        let edge_count = incoming_data.get(&node.id).copied().unwrap_or(0);

        // --- Tier 1: op-specific checks ---
        match node.op {
            AISOperationType::Ask | AISOperationType::Think | AISOperationType::Reason => {
                // E501: template placeholder bounds
                check_placeholder_bounds(
                    node,
                    graph_attrs::TEMPLATE_STR,
                    edge_count,
                    ErrorCode::TemplatePlaceholderBounds,
                    &mut errors,
                );
            }
            AISOperationType::Communicate => {
                // E502: recipient not spawned (ACP protocol only)
                let protocol = node
                    .attributes
                    .get(graph_attrs::PROTOCOL)
                    .and_then(|v| v.as_str());
                if protocol == Some(communicate_protocols::ACP) {
                    if let Some(recipient) = node
                        .attributes
                        .get(graph_attrs::RECIPIENT)
                        .and_then(|v| v.as_str())
                    {
                        if !spawned_agents.contains_key(recipient) {
                            errors.push(Error::new_global(
                                ErrorCode::CommunicateRecipientNotSpawned,
                                format!(
                                    "node '{}': COMMUNICATE recipient '{}' not spawned by any SPAWN_AGENT in this graph",
                                    node.name, recipient
                                ),
                                &node.name,
                            ));
                        }
                    }
                }
            }
            AISOperationType::InvTool => {
                let cap_name = node
                    .attributes
                    .get(graph_attrs::CAPABILITY)
                    .and_then(|v| v.as_str());

                // E504: params_json placeholder bounds
                check_placeholder_bounds(
                    node,
                    graph_attrs::PARAMS_JSON,
                    edge_count,
                    ErrorCode::InvPlaceholderBounds,
                    &mut errors,
                );

                // E507: INV(acp) agent reference (legacy pattern)
                if has_ctx && cap_name == Some(communicate_protocols::ACP) {
                    if let Some(pj) = node
                        .attributes
                        .get(graph_attrs::PARAMS_JSON)
                        .and_then(|v| v.as_str())
                    {
                        check_inv_acp_agent(pj, ctx, node, &mut errors);
                    }
                }
            }
            _ => {}
        }

        // --- Tier 2: generic ref_type checks (data-driven) ---
        if has_ctx {
            for field in spec.fields {
                let Some(ref_type) = field.ref_type else {
                    continue;
                };
                let Some(value) = node.attributes.get(field.name) else {
                    continue;
                };
                let Some(name) = value.as_str() else {
                    continue;
                };

                // INV(acp) uses "acp" as capability but it's a protocol, not a tool
                if ref_type == ReferenceType::Capability && name == communicate_protocols::ACP {
                    continue;
                }

                if !ctx.contains(ref_type, name) {
                    errors.push(make_ref_error(ref_type, node, name));
                }
            }
        }
    }

    // E505: parameter/entry-node arity
    check_parameter_arity(graph, &mut errors);

    // E511: spawn_agent in parameterized flow
    check_spawn_agent_in_parameterized_flow(graph, &mut errors);

    errors
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn count_incoming_data_edges(graph: &ApxmGraph) -> HashMap<u64, usize> {
    let mut counts = HashMap::new();
    for edge in &graph.edges {
        if edge.dependency == DependencyType::Data {
            *counts.entry(edge.to).or_insert(0) += 1;
        }
    }
    counts
}

/// Parse `{N}` placeholders from a string and return the maximum index found.
fn max_placeholder_index(s: &str) -> Option<usize> {
    let mut max = None;
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'{' {
            let start = i + 1;
            let mut end = start;
            while end < bytes.len() && bytes[end].is_ascii_digit() {
                end += 1;
            }
            if end > start && end < bytes.len() && bytes[end] == b'}' {
                if let Ok(idx) = s[start..end].parse::<usize>() {
                    max = Some(max.map_or(idx, |m: usize| m.max(idx)));
                }
            }
            i = end;
        } else {
            i += 1;
        }
    }
    max
}

fn check_placeholder_bounds(
    node: &GraphNode,
    attr_name: &str,
    edge_count: usize,
    code: ErrorCode,
    errors: &mut Vec<Error>,
) {
    if let Some(tpl) = node.attributes.get(attr_name).and_then(|v| v.as_str()) {
        if let Some(max_idx) = max_placeholder_index(tpl) {
            if max_idx >= edge_count {
                errors.push(Error::new_global(
                    code,
                    format!(
                        "node '{}' ({}): placeholder {{{}}} requires {} data input(s), but node has {}",
                        node.name, node.op, max_idx, max_idx + 1, edge_count
                    ),
                    &node.name,
                ));
            }
        }
    }
}

/// Check that spawn_agent is not used in a graph that has flow parameters.
///
/// When a graph declares parameters, the MLIR lowering injects `%arg0..N`
/// as inputs to entry nodes (nodes with no incoming edges). spawn_agent
/// is an entry node in most graphs, and its MLIR emitter does not expect
/// these injected inputs — producing a cryptic E900 parse error.
///
/// This check surfaces the incompatibility as a clear E511 error with
/// actionable guidance before lowering is attempted.
fn check_spawn_agent_in_parameterized_flow(graph: &ApxmGraph, errors: &mut Vec<Error>) {
    if graph.parameters.is_empty() {
        return; // No parameters — no conflict possible.
    }

    let has_spawn = graph
        .nodes
        .iter()
        .any(|n| n.op == AISOperationType::SpawnAgent);

    if has_spawn {
        let param_names: Vec<&str> = graph.parameters.iter().map(|p| p.name.as_str()).collect();
        errors.push(
            Error::new_global(
                ErrorCode::SpawnAgentInParameterizedFlow,
                format!(
                    "graph '{}' uses spawn_agent but also declares flow parameters [{}]. \
                     spawn_agent cannot be used in a flow with parameters.",
                    graph.name,
                    param_names.join(", ")
                ),
                &graph.name,
            )
            .with_help(
                "Move the task input inside the flow body using ask() or const_str(), \
                 or restructure so spawn_agent lives in a separate no-parameter @entry flow \
                 and the parameterized logic lives in a helper flow of another agent. \
                 See docs/bugs/compiler-spawn-agent-flow-params.md for details."
                    .to_string(),
            ),
        );
    }
}

fn check_parameter_arity(graph: &ApxmGraph, errors: &mut Vec<Error>) {
    if graph.parameters.is_empty() {
        return;
    }

    let targets: HashSet<u64> = graph.edges.iter().map(|e| e.to).collect();
    let entry_count = graph
        .nodes
        .iter()
        .filter(|n| !targets.contains(&n.id))
        .count();

    let param_count = graph.parameters.len();
    if param_count > 0 && entry_count > 0 && param_count != entry_count {
        errors.push(
            Error::new_global(
                ErrorCode::ParameterArityMismatch,
                format!(
                    "graph declares {} parameter(s) but has {} entry node(s)",
                    param_count, entry_count
                ),
                &graph.name,
            )
            .with_help(format!(
                "Each parameter is injected into an entry node. Expected {} parameter(s) to match {} entry node(s).",
                entry_count, entry_count
            )),
        );
    }
}

/// Check if INV(acp) params_json references an agent not in the profile registry.
fn check_inv_acp_agent(
    params_json: &str,
    ctx: &SemanticContext,
    node: &GraphNode,
    errors: &mut Vec<Error>,
) {
    // Try to parse the params_json to find an "agent" field
    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(params_json) {
        if let Some(agent_name) = parsed.get("agent").and_then(|v| v.as_str()) {
            if !ctx.profiles.contains(agent_name) {
                errors.push(
                    Error::new_global(
                        ErrorCode::InvAcpAgentNotRegistered,
                        format!(
                            "node '{}' (INV/acp): agent '{}' is not a registered profile",
                            node.name, agent_name
                        ),
                        &node.name,
                    )
                    .with_suggestion(Suggestion::new(
                        "Run `apxm agent list` to see available profiles".to_string(),
                    )),
                );
            }
        }
    }
}

fn ref_error_code(ref_type: ReferenceType) -> ErrorCode {
    match ref_type {
        ReferenceType::Profile => ErrorCode::ProfileNotRegistered,
        ReferenceType::Backend => ErrorCode::BackendNotRegistered,
        ReferenceType::Model => ErrorCode::ModelNotFound,
        ReferenceType::Capability => ErrorCode::CapabilityNotRegistered,
    }
}

fn make_ref_error(ref_type: ReferenceType, node: &GraphNode, name: &str) -> Error {
    Error::new_global(
        ref_error_code(ref_type),
        format!(
            "node '{}' ({}): {} '{}' is not registered",
            node.name,
            node.op,
            ref_type.label(),
            name
        ),
        &node.name,
    )
    .with_suggestion(Suggestion::new(format!(
        "Run `{}` to see available options",
        ref_type.list_command()
    )))
    .with_help(format!(
        "To register: `{} {} ...`",
        ref_type.add_command(),
        name
    ))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{GraphEdge, Parameter};

    fn make_node(id: u64, name: &str, op: AISOperationType) -> GraphNode {
        GraphNode {
            id,
            name: name.to_string(),
            op,
            attributes: HashMap::new(),
        }
    }

    fn make_graph(nodes: Vec<GraphNode>, edges: Vec<GraphEdge>) -> ApxmGraph {
        ApxmGraph {
            name: "test".to_string(),
            nodes,
            edges,
            parameters: Vec::new(),
            metadata: HashMap::new(),
        }
    }

    fn data_edge(from: u64, to: u64) -> GraphEdge {
        GraphEdge {
            from,
            to,
            dependency: DependencyType::Data,
        }
    }

    // --- Tier 1 Tests ---

    #[test]
    fn e501_template_placeholder_out_of_bounds() {
        let mut node = make_node(2, "ask", AISOperationType::Ask);
        node.attributes.insert(
            graph_attrs::TEMPLATE_STR.to_string(),
            apxm_core::types::Value::String("{1}".to_string()),
        );
        let const_node = make_node(1, "const", AISOperationType::ConstStr);
        let graph = make_graph(vec![const_node, node], vec![data_edge(1, 2)]);
        let errors = validate_semantic(&graph, &SemanticContext::default());
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, ErrorCode::TemplatePlaceholderBounds);
    }

    #[test]
    fn e501_valid_placeholder_no_error() {
        let mut node = make_node(2, "ask", AISOperationType::Ask);
        node.attributes.insert(
            graph_attrs::TEMPLATE_STR.to_string(),
            apxm_core::types::Value::String("{0}".to_string()),
        );
        let const_node = make_node(1, "const", AISOperationType::ConstStr);
        let graph = make_graph(vec![const_node, node], vec![data_edge(1, 2)]);
        let errors = validate_semantic(&graph, &SemanticContext::default());
        assert!(errors.is_empty());
    }

    #[test]
    fn e502_communicate_recipient_not_spawned() {
        let mut comm = make_node(1, "send", AISOperationType::Communicate);
        comm.attributes.insert(
            graph_attrs::PROTOCOL.to_string(),
            apxm_core::types::Value::String("acp".to_string()),
        );
        comm.attributes.insert(
            graph_attrs::RECIPIENT.to_string(),
            apxm_core::types::Value::String("coder".to_string()),
        );
        let graph = make_graph(vec![comm], vec![]);
        let errors = validate_semantic(&graph, &SemanticContext::default());
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, ErrorCode::CommunicateRecipientNotSpawned);
    }

    #[test]
    fn e502_local_protocol_not_checked() {
        let mut comm = make_node(1, "send", AISOperationType::Communicate);
        comm.attributes.insert(
            graph_attrs::PROTOCOL.to_string(),
            apxm_core::types::Value::String("local".to_string()),
        );
        comm.attributes.insert(
            graph_attrs::RECIPIENT.to_string(),
            apxm_core::types::Value::String("nobody".to_string()),
        );
        let graph = make_graph(vec![comm], vec![]);
        let errors = validate_semantic(&graph, &SemanticContext::default());
        assert!(errors.is_empty());
    }

    #[test]
    fn e503_duplicate_agent_name() {
        let mut spawn1 = make_node(1, "spawn1", AISOperationType::SpawnAgent);
        spawn1.attributes.insert(
            graph_attrs::AGENT_NAME.to_string(),
            apxm_core::types::Value::String("coder".to_string()),
        );
        let mut spawn2 = make_node(2, "spawn2", AISOperationType::SpawnAgent);
        spawn2.attributes.insert(
            graph_attrs::AGENT_NAME.to_string(),
            apxm_core::types::Value::String("coder".to_string()),
        );
        let graph = make_graph(vec![spawn1, spawn2], vec![]);
        let errors = validate_semantic(&graph, &SemanticContext::default());
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, ErrorCode::DuplicateAgentName);
    }

    #[test]
    fn e504_inv_placeholder_bounds() {
        let mut inv = make_node(2, "inv", AISOperationType::InvTool);
        inv.attributes.insert(
            graph_attrs::CAPABILITY.to_string(),
            apxm_core::types::Value::String("bash".to_string()),
        );
        inv.attributes.insert(
            graph_attrs::PARAMS_JSON.to_string(),
            apxm_core::types::Value::String(r#"{"cmd": "{2}"}"#.to_string()),
        );
        let const_node = make_node(1, "const", AISOperationType::ConstStr);
        let graph = make_graph(vec![const_node, inv], vec![data_edge(1, 2)]);
        let errors = validate_semantic(&graph, &SemanticContext::default());
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, ErrorCode::InvPlaceholderBounds);
    }

    #[test]
    fn e505_parameter_arity_mismatch() {
        let node = make_node(1, "ask", AISOperationType::Ask);
        let mut graph = make_graph(vec![node], vec![]);
        graph.parameters = vec![
            Parameter {
                name: "a".to_string(),
                type_name: "str".to_string(),
            },
            Parameter {
                name: "b".to_string(),
                type_name: "str".to_string(),
            },
        ];
        let errors = validate_semantic(&graph, &SemanticContext::default());
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, ErrorCode::ParameterArityMismatch);
    }

    #[test]
    fn valid_graph_no_errors() {
        let mut ask = make_node(2, "ask", AISOperationType::Ask);
        ask.attributes.insert(
            graph_attrs::TEMPLATE_STR.to_string(),
            apxm_core::types::Value::String("{0}".to_string()),
        );
        let const_node = make_node(1, "const", AISOperationType::ConstStr);
        let graph = make_graph(vec![const_node, ask], vec![data_edge(1, 2)]);
        let errors = validate_semantic(&graph, &SemanticContext::default());
        assert!(errors.is_empty(), "expected no errors, got: {:?}", errors);
    }

    // --- Tier 2 Tests ---

    #[test]
    fn e506_profile_not_registered() {
        let mut spawn = make_node(1, "spawn", AISOperationType::SpawnAgent);
        spawn.attributes.insert(
            graph_attrs::AGENT_NAME.to_string(),
            apxm_core::types::Value::String("reviewer".to_string()),
        );
        spawn.attributes.insert(
            graph_attrs::PROFILE.to_string(),
            apxm_core::types::Value::String("codex".to_string()),
        );
        let graph = make_graph(vec![spawn], vec![]);

        let mut ctx = SemanticContext::default();
        ctx.profiles.insert(ProfileId::from("claude"));
        // "codex" not in ctx

        let errors = validate_semantic(&graph, &ctx);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, ErrorCode::ProfileNotRegistered);
    }

    #[test]
    fn e510_capability_not_registered() {
        let mut inv = make_node(1, "inv", AISOperationType::InvTool);
        inv.attributes.insert(
            graph_attrs::CAPABILITY.to_string(),
            apxm_core::types::Value::String("unknown_tool".to_string()),
        );
        let graph = make_graph(vec![inv], vec![]);

        let mut ctx = SemanticContext::default();
        ctx.capabilities.insert(CapabilityName::from("bash"));

        let errors = validate_semantic(&graph, &ctx);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, ErrorCode::CapabilityNotRegistered);
    }

    #[test]
    fn e509_model_not_found() {
        let mut ask = make_node(1, "ask", AISOperationType::Ask);
        ask.attributes.insert(
            graph_attrs::TEMPLATE_STR.to_string(),
            apxm_core::types::Value::String("hello".to_string()),
        );
        ask.attributes.insert(
            graph_attrs::MODEL.to_string(),
            apxm_core::types::Value::String("nonexistent".to_string()),
        );
        let graph = make_graph(vec![ask], vec![]);

        let mut ctx = SemanticContext::default();
        ctx.models.insert(ModelId::from("gpt-4"));

        let errors = validate_semantic(&graph, &ctx);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, ErrorCode::ModelNotFound);
    }

    #[test]
    fn tier2_skipped_with_empty_context() {
        let mut spawn = make_node(1, "spawn", AISOperationType::SpawnAgent);
        spawn.attributes.insert(
            graph_attrs::AGENT_NAME.to_string(),
            apxm_core::types::Value::String("reviewer".to_string()),
        );
        spawn.attributes.insert(
            graph_attrs::PROFILE.to_string(),
            apxm_core::types::Value::String("codex".to_string()),
        );
        let graph = make_graph(vec![spawn], vec![]);

        // Empty context = Tier 2 checks don't run
        let errors = validate_semantic(&graph, &SemanticContext::default());
        assert!(errors.is_empty());
    }

    #[test]
    fn valid_graph_with_full_context() {
        let mut ask = make_node(2, "ask", AISOperationType::Ask);
        ask.attributes.insert(
            graph_attrs::TEMPLATE_STR.to_string(),
            apxm_core::types::Value::String("{0}".to_string()),
        );
        ask.attributes.insert(
            graph_attrs::MODEL.to_string(),
            apxm_core::types::Value::String("gpt-4".to_string()),
        );
        let const_node = make_node(1, "const", AISOperationType::ConstStr);
        let graph = make_graph(vec![const_node, ask], vec![data_edge(1, 2)]);

        let mut ctx = SemanticContext::default();
        ctx.models.insert(ModelId::from("gpt-4"));

        let errors = validate_semantic(&graph, &ctx);
        assert!(errors.is_empty(), "expected no errors, got: {:?}", errors);
    }

    #[test]
    fn inv_acp_capability_not_flagged_as_tool() {
        // INV(acp) should NOT trigger E510 since "acp" is a protocol dispatch shortcut
        let mut inv = make_node(1, "inv", AISOperationType::InvTool);
        inv.attributes.insert(
            graph_attrs::CAPABILITY.to_string(),
            apxm_core::types::Value::String("acp".to_string()),
        );
        let graph = make_graph(vec![inv], vec![]);

        let mut ctx = SemanticContext::default();
        ctx.capabilities.insert(CapabilityName::from("bash"));

        let errors = validate_semantic(&graph, &ctx);
        assert!(errors.is_empty());
    }

    // --- Placeholder parser tests ---

    #[test]
    fn max_placeholder_basic() {
        assert_eq!(max_placeholder_index("{0}"), Some(0));
        assert_eq!(max_placeholder_index("{0} and {2}"), Some(2));
        assert_eq!(max_placeholder_index("no placeholders"), None);
        assert_eq!(max_placeholder_index("{abc}"), None);
        assert_eq!(max_placeholder_index("{}"), None);
        assert_eq!(max_placeholder_index("{10}"), Some(10));
    }

    // ── E511: spawn_agent in parameterized flow ────────────────────────────

    #[test]
    fn e511_spawn_agent_with_flow_params_is_error() {
        

        let mut spawn = make_node(1, "spawn", AISOperationType::SpawnAgent);
        spawn.attributes.insert(
            graph_attrs::AGENT_NAME.to_string(),
            apxm_core::types::Value::String("coder".to_string()),
        );

        let mut graph = make_graph(vec![spawn], vec![]);
        graph.parameters = vec![crate::Parameter {
            name: "TASK".to_string(),
            type_name: "str".to_string(),
        }];

        let errors = validate_semantic(&graph, &SemanticContext::default());
        let e511 = errors
            .iter()
            .find(|e| e.code == ErrorCode::SpawnAgentInParameterizedFlow);
        assert!(
            e511.is_some(),
            "expected E511 but got: {:?}",
            errors.iter().map(|e| e.code.as_str()).collect::<Vec<_>>()
        );
        assert!(!e511.unwrap().code.is_warning(), "E511 should be a hard error");
        // Help text should mention the workaround
        let help = e511.unwrap().help.as_deref().unwrap_or("");
        assert!(help.contains("ask()"), "help should mention ask() workaround");
    }

    #[test]
    fn e511_not_triggered_without_params() {
        let mut spawn = make_node(1, "spawn", AISOperationType::SpawnAgent);
        spawn.attributes.insert(
            graph_attrs::AGENT_NAME.to_string(),
            apxm_core::types::Value::String("coder".to_string()),
        );

        let graph = make_graph(vec![spawn], vec![]);
        // No parameters set — should not trigger E511.
        let errors = validate_semantic(&graph, &SemanticContext::default());
        let e511 = errors
            .iter()
            .find(|e| e.code == ErrorCode::SpawnAgentInParameterizedFlow);
        assert!(e511.is_none(), "E511 should not fire when graph has no parameters");
    }

    #[test]
    fn e511_not_triggered_without_spawn_agent() {
        

        let mut ask = make_node(1, "ask", AISOperationType::Ask);
        ask.attributes.insert(
            graph_attrs::TEMPLATE_STR.to_string(),
            apxm_core::types::Value::String("hello".to_string()),
        );

        let mut graph = make_graph(vec![ask], vec![]);
        graph.parameters = vec![crate::Parameter {
            name: "TASK".to_string(),
            type_name: "str".to_string(),
        }];

        // Has parameters but no spawn_agent — should not trigger E511.
        let errors = validate_semantic(&graph, &SemanticContext::default());
        let e511 = errors
            .iter()
            .find(|e| e.code == ErrorCode::SpawnAgentInParameterizedFlow);
        assert!(e511.is_none(), "E511 should not fire when graph has no spawn_agent");
    }
}
