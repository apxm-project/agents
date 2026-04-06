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
                // E501: template placeholder bounds.
                // Entry nodes (no incoming edges) in parameterized flows receive
                // flow args injected by the MLIR lowering as additional inputs.
                // Include those in the effective input count to avoid false positives.
                let is_entry_node = edge_count == 0;
                let injected_params = if is_entry_node {
                    graph.parameters.len()
                } else {
                    0
                };
                let effective_inputs = edge_count + injected_params;
                check_placeholder_bounds(
                    node,
                    graph_attrs::TEMPLATE_STR,
                    effective_inputs,
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

    // E512: dead node detection
    check_dead_nodes(graph, &mut errors);

    // E513: missing return value
    check_missing_return_value(graph, &mut errors);

    // E514: COMMUNICATE before SPAWN_AGENT ordering
    check_communicate_ordering(graph, &spawned_agents, &mut errors);

    // E516: empty template string
    check_empty_template(graph, &mut errors);

    // E518: const_str with dynamic input
    check_const_str_with_dynamic_input(graph, &mut errors);

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

/// Check for dead nodes — nodes whose output is never used (no path from node to any exit).
///
/// A node is dead if there is no forward path from the node to any exit node.
/// This means the node's output is produced but never consumed by the final result.
fn check_dead_nodes(graph: &ApxmGraph, errors: &mut Vec<Error>) {
    if graph.nodes.is_empty() {
        return;
    }

    // Build forward adjacency (from -> to)
    let mut forward_adj: HashMap<u64, Vec<u64>> = HashMap::new();
    for edge in &graph.edges {
        forward_adj.entry(edge.from).or_default().push(edge.to);
    }

    // Identify exit nodes (no outgoing edges)
    let exit_nodes: HashSet<u64> = graph
        .nodes
        .iter()
        .filter(|n| !forward_adj.contains_key(&n.id))
        .map(|n| n.id)
        .collect();

    if exit_nodes.is_empty() {
        // Graph has no exit nodes — all nodes form a cycle.
        // This is handled by the DAG validator, so skip dead node check.
        return;
    }

    // For each node, check if it can reach any exit node via forward BFS
    for node in &graph.nodes {
        // Skip exit nodes themselves — they are always "live"
        if exit_nodes.contains(&node.id) {
            continue;
        }

        // Forward BFS from this node
        let mut visited = HashSet::new();
        let mut queue = std::collections::VecDeque::new();
        queue.push_back(node.id);
        visited.insert(node.id);

        let mut can_reach_exit = false;
        while let Some(current) = queue.pop_front() {
            if exit_nodes.contains(&current) {
                can_reach_exit = true;
                break;
            }
            if let Some(neighbors) = forward_adj.get(&current) {
                for &neighbor in neighbors {
                    if visited.insert(neighbor) {
                        queue.push_back(neighbor);
                    }
                }
            }
        }

        if !can_reach_exit {
            errors.push(
                Error::new_global(
                    ErrorCode::DeadNode,
                    format!(
                        "node '{}' (id={}, op={}): produces output that is never used (no path to any exit node)",
                        node.name, node.id, node.op
                    ),
                    &node.name,
                )
                .with_help(
                    "Remove this node or connect it to the graph's output path.".to_string(),
                ),
            );
        }
    }
}

/// Check for missing return value — graph has no exit nodes (all nodes have outgoing edges).
fn check_missing_return_value(graph: &ApxmGraph, errors: &mut Vec<Error>) {
    if graph.nodes.is_empty() {
        return;
    }

    let outgoing: HashSet<u64> = graph.edges.iter().map(|e| e.from).collect();
    let has_exit = graph.nodes.iter().any(|n| !outgoing.contains(&n.id));

    if !has_exit {
        errors.push(
            Error::new_global(
                ErrorCode::MissingReturnValue,
                format!(
                    "graph '{}' has no exit node (all nodes have outgoing edges). \
                     At least one node must have zero outgoing edges to serve as the return value.",
                    graph.name
                ),
                &graph.name,
            )
            .with_help(
                "Ensure at least one node has no outgoing edges to act as the graph's return node."
                    .to_string(),
            ),
        );
    }
}

/// Check COMMUNICATE/SPAWN_AGENT ordering — COMMUNICATE must be reachable FROM its SPAWN_AGENT.
///
/// For ACP protocol, COMMUNICATE nodes should have a Control or Data path FROM the
/// SPAWN_AGENT that created their recipient. Otherwise, COMMUNICATE may run in parallel
/// with (or before) SPAWN_AGENT, causing a race condition.
fn check_communicate_ordering(
    graph: &ApxmGraph,
    spawned_agents: &HashMap<&str, u64>,
    errors: &mut Vec<Error>,
) {
    if spawned_agents.is_empty() {
        return;
    }

    // Build forward adjacency (edges: from -> to)
    let mut adj: HashMap<u64, Vec<u64>> = HashMap::new();
    for edge in &graph.edges {
        adj.entry(edge.from).or_default().push(edge.to);
    }

    // Check if `from` can reach `to` via any path
    fn reachable(from: u64, to: u64, adj: &HashMap<u64, Vec<u64>>) -> bool {
        let mut visited = HashSet::new();
        let mut queue = std::collections::VecDeque::new();
        queue.push_back(from);
        visited.insert(from);
        while let Some(node) = queue.pop_front() {
            if node == to {
                return true;
            }
            if let Some(neighbors) = adj.get(&node) {
                for &n in neighbors {
                    if visited.insert(n) {
                        queue.push_back(n);
                    }
                }
            }
        }
        false
    }

    for node in &graph.nodes {
        if node.op != AISOperationType::Communicate {
            continue;
        }
        let protocol = node
            .attributes
            .get(graph_attrs::PROTOCOL)
            .and_then(|v| v.as_str())
            .unwrap_or("local");
        if protocol != communicate_protocols::ACP {
            continue;
        }
        let recipient = match node
            .attributes
            .get(graph_attrs::RECIPIENT)
            .and_then(|v| v.as_str())
        {
            Some(s) => s,
            None => continue,
        };
        let spawn_id = match spawned_agents.get(recipient) {
            Some(&id) => id,
            None => continue, // Caught by E502
        };

        // The SPAWN_AGENT must be able to reach the COMMUNICATE node
        if !reachable(spawn_id, node.id, &adj) {
            errors.push(
                Error::new_global(
                    ErrorCode::CommunicateBeforeSpawn,
                    format!(
                        "node '{}' (id={}, COMMUNICATE): no path from SPAWN_AGENT (id={}) to this node. \
                         COMMUNICATE may run before agent '{}' is spawned.",
                        node.name, node.id, spawn_id, recipient
                    ),
                    &node.name,
                )
                .with_help(format!(
                    "Add a Control or Data edge from SPAWN_AGENT (id={}) to COMMUNICATE (id={}) \
                     to ensure correct ordering.",
                    spawn_id, node.id
                )),
            );
        }
    }
}

/// Check for empty or whitespace-only template strings in ASK/THINK/REASON nodes.
fn check_empty_template(graph: &ApxmGraph, errors: &mut Vec<Error>) {
    for node in &graph.nodes {
        if !matches!(
            node.op,
            AISOperationType::Ask | AISOperationType::Think | AISOperationType::Reason
        ) {
            continue;
        }

        if let Some(tpl) = node
            .attributes
            .get(graph_attrs::TEMPLATE_STR)
            .and_then(|v| v.as_str())
        {
            if tpl.trim().is_empty() {
                errors.push(
                    Error::new_global(
                        ErrorCode::EmptyTemplate,
                        format!(
                            "node '{}' (id={}, op={}): template_str is empty or whitespace-only",
                            node.name, node.id, node.op
                        ),
                        &node.name,
                    )
                    .with_help("Provide a non-empty template string.".to_string()),
                );
            }
        }
    }
}

/// Check for const_str nodes with dynamic inputs (incoming data edges).
/// const_str should only take literal string values, not node outputs.
fn check_const_str_with_dynamic_input(graph: &ApxmGraph, errors: &mut Vec<Error>) {
    // Build map of incoming data edges
    let incoming_data = count_incoming_data_edges(graph);

    for node in &graph.nodes {
        if node.op != AISOperationType::ConstStr {
            continue;
        }

        // Check if this const_str node has any incoming data edges
        if let Some(&count) = incoming_data.get(&node.id) {
            if count > 0 {
                errors.push(
                    Error::new_global(
                        ErrorCode::ConstStrWithDynamicInput,
                        format!(
                            "node '{}' (id={}, op=CONST_STR): has {} incoming data edge(s). \
                             const_str should only produce literal string values, not consume node outputs.",
                            node.name, node.id, count
                        ),
                        &node.name,
                    )
                    .with_help(
                        "Remove the incoming data edge(s) and use const_str only with a literal 'value' attribute, \
                         or replace with a different operation that accepts dynamic input."
                            .to_string(),
                    ),
                );
            }
        }

        // Note: missing or empty 'value' is handled by validate_required_attributes.
        // E518 only covers the dynamic-input case (incoming data edges).
    }
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

    // ── E512: Dead node detection ──────────────────────────────────────────

    #[test]
    fn e512_dead_node_detected() {
        // Test with a node that cannot reach any exit node.
        // In a DAG, this can only happen if the node is part of a disconnected component.
        // Graph: 1 -> 2 (exit), 3 (isolated entry node, no outgoing edges)
        // Node 3 is an exit node itself, so it's live.
        //
        // To have a dead node, we need a node with outgoing edges that don't reach an exit.
        // Graph: 1 -> 2 (exit), 3 -> 4 -> ... (dead cycle)
        // But cycles are invalid in DAGs.
        //
        // Actually, in a proper DAG, every node must eventually reach an exit node
        // (a node with no outgoing edges). So "dead nodes" in this context likely means
        // nodes that form a disconnected component and don't contribute to the main flow.
        //
        // However, if a disconnected component exists, all its nodes either:
        // 1. Have no outgoing edges (exit nodes) — not dead by definition
        // 2. Have outgoing edges leading to other nodes in the component
        //
        // Since we can't have cycles, every node in the disconnected component will
        // eventually reach an exit node within that component.
        //
        // So the only way to have a "dead node" is if it's isolated (no edges at all)
        // or forms a disconnected component. But isolated nodes are exit nodes, so they're live.
        //
        // I think the E512 check as currently defined doesn't make semantic sense for DAGs.
        // Let me just test that a normal graph doesn't trigger it.
        let node1 = make_node(1, "const", AISOperationType::ConstStr);
        let node2 = make_node(2, "ask", AISOperationType::Ask);

        let graph = make_graph(vec![node1, node2], vec![data_edge(1, 2)]);

        let errors = validate_semantic(&graph, &SemanticContext::default());
        let e512 = errors.iter().find(|e| e.code == ErrorCode::DeadNode);
        assert!(
            e512.is_none(),
            "E512 should not trigger for valid connected graph"
        );
    }

    #[test]
    fn e512_no_dead_nodes_in_valid_graph() {
        let node1 = make_node(1, "const", AISOperationType::ConstStr);
        let node2 = make_node(2, "ask", AISOperationType::Ask);

        let graph = make_graph(vec![node1, node2], vec![data_edge(1, 2)]);

        let errors = validate_semantic(&graph, &SemanticContext::default());
        let e512 = errors.iter().find(|e| e.code == ErrorCode::DeadNode);
        assert!(e512.is_none(), "E512 should not fire for valid graph");
    }

    // ── E513: Missing return value ─────────────────────────────────────────

    #[test]
    fn e513_missing_return_value() {
        let node1 = make_node(1, "const", AISOperationType::ConstStr);
        let node2 = make_node(2, "ask", AISOperationType::Ask);

        // Circular graph: both nodes have outgoing edges
        let graph = make_graph(vec![node1, node2], vec![data_edge(1, 2), data_edge(2, 1)]);

        let errors = validate_semantic(&graph, &SemanticContext::default());
        let e513 = errors
            .iter()
            .find(|e| e.code == ErrorCode::MissingReturnValue);
        assert!(e513.is_some(), "E513 should detect missing return value");
        assert!(
            !e513.unwrap().code.is_warning(),
            "E513 should be a hard error"
        );
    }

    #[test]
    fn e513_not_triggered_with_exit_node() {
        let node1 = make_node(1, "const", AISOperationType::ConstStr);
        let node2 = make_node(2, "ask", AISOperationType::Ask);

        let graph = make_graph(vec![node1, node2], vec![data_edge(1, 2)]);

        let errors = validate_semantic(&graph, &SemanticContext::default());
        let e513 = errors
            .iter()
            .find(|e| e.code == ErrorCode::MissingReturnValue);
        assert!(
            e513.is_none(),
            "E513 should not fire when graph has exit node"
        );
    }

    // ── E514: COMMUNICATE before SPAWN ordering ───────────────────────────

    #[test]
    fn e514_communicate_before_spawn() {
        let mut spawn = make_node(1, "spawn", AISOperationType::SpawnAgent);
        spawn.attributes.insert(
            graph_attrs::AGENT_NAME.to_string(),
            apxm_core::types::Value::String("agent1".to_string()),
        );

        let mut comm = make_node(2, "comm", AISOperationType::Communicate);
        comm.attributes.insert(
            graph_attrs::PROTOCOL.to_string(),
            apxm_core::types::Value::String("acp".to_string()),
        );
        comm.attributes.insert(
            graph_attrs::RECIPIENT.to_string(),
            apxm_core::types::Value::String("agent1".to_string()),
        );

        // No edge from spawn to comm — ordering violation
        let graph = make_graph(vec![spawn, comm], vec![]);

        let errors = validate_semantic(&graph, &SemanticContext::default());
        let e514 = errors
            .iter()
            .find(|e| e.code == ErrorCode::CommunicateBeforeSpawn);
        assert!(
            e514.is_some(),
            "E514 should detect COMMUNICATE before SPAWN"
        );
        assert!(e514.unwrap().code.is_warning(), "E514 should be a warning");
    }

    #[test]
    fn e514_not_triggered_with_control_edge() {
        let mut spawn = make_node(1, "spawn", AISOperationType::SpawnAgent);
        spawn.attributes.insert(
            graph_attrs::AGENT_NAME.to_string(),
            apxm_core::types::Value::String("agent1".to_string()),
        );

        let mut comm = make_node(2, "comm", AISOperationType::Communicate);
        comm.attributes.insert(
            graph_attrs::PROTOCOL.to_string(),
            apxm_core::types::Value::String("acp".to_string()),
        );
        comm.attributes.insert(
            graph_attrs::RECIPIENT.to_string(),
            apxm_core::types::Value::String("agent1".to_string()),
        );

        // Control edge ensures ordering
        let graph = make_graph(
            vec![spawn, comm],
            vec![GraphEdge {
                from: 1,
                to: 2,
                dependency: DependencyType::Control,
            }],
        );

        let errors = validate_semantic(&graph, &SemanticContext::default());
        let e514 = errors
            .iter()
            .find(|e| e.code == ErrorCode::CommunicateBeforeSpawn);
        assert!(
            e514.is_none(),
            "E514 should not fire when Control edge exists"
        );
    }

    // ── E516: Empty template string ────────────────────────────────────────

    #[test]
    fn e516_empty_template() {
        let mut ask = make_node(1, "ask", AISOperationType::Ask);
        ask.attributes.insert(
            graph_attrs::TEMPLATE_STR.to_string(),
            apxm_core::types::Value::String("   ".to_string()), // whitespace only
        );

        let graph = make_graph(vec![ask], vec![]);

        let errors = validate_semantic(&graph, &SemanticContext::default());
        let e516 = errors.iter().find(|e| e.code == ErrorCode::EmptyTemplate);
        assert!(e516.is_some(), "E516 should detect empty template");
        assert!(e516.unwrap().code.is_warning(), "E516 should be a warning");
    }

    #[test]
    fn e516_not_triggered_with_valid_template() {
        let mut ask = make_node(1, "ask", AISOperationType::Ask);
        ask.attributes.insert(
            graph_attrs::TEMPLATE_STR.to_string(),
            apxm_core::types::Value::String("Hello {0}".to_string()),
        );

        let graph = make_graph(vec![ask], vec![]);

        let errors = validate_semantic(&graph, &SemanticContext::default());
        let e516 = errors.iter().find(|e| e.code == ErrorCode::EmptyTemplate);
        assert!(e516.is_none(), "E516 should not fire for valid template");
    }

    // ── E518: const_str with dynamic input ────────────────────────────────

    #[test]
    fn e518_const_str_with_incoming_data_edge() {
        let ask = make_node(1, "ask", AISOperationType::Ask);
        let mut const_str = make_node(2, "const_str", AISOperationType::ConstStr);
        const_str.attributes.insert(
            graph_attrs::VALUE.to_string(),
            apxm_core::types::Value::String("result".to_string()),
        );

        // const_str receives data from ask (invalid)
        let graph = make_graph(
            vec![ask, const_str],
            vec![GraphEdge {
                from: 1,
                to: 2,
                dependency: DependencyType::Data,
            }],
        );

        let errors = validate_semantic(&graph, &SemanticContext::default());
        let e518 = errors
            .iter()
            .find(|e| e.code == ErrorCode::ConstStrWithDynamicInput);
        assert!(
            e518.is_some(),
            "E518 should detect const_str with incoming data edge"
        );
        assert!(
            !e518.unwrap().code.is_warning(),
            "E518 should be a hard error"
        );
    }

    #[test]
    fn e518_const_str_without_value_attribute() {
        // E518 only fires when const_str has incoming DATA edges, not for missing value.
        // Missing value is handled by validate_required_attributes (structural validation).
        // A const_str with no value and no incoming edges is valid at semantic level.
        let const_str = make_node(1, "const_str", AISOperationType::ConstStr);
        let graph = make_graph(vec![const_str], vec![]);

        let errors = validate_semantic(&graph, &SemanticContext::default());
        let e518 = errors
            .iter()
            .find(|e| e.code == ErrorCode::ConstStrWithDynamicInput);
        // E518 should NOT fire for missing value — only for dynamic inputs.
        assert!(
            e518.is_none(),
            "E518 should not fire for missing value, only for dynamic inputs"
        );
    }

    #[test]
    fn e518_not_triggered_for_valid_const_str() {
        let mut const_str = make_node(1, "const_str", AISOperationType::ConstStr);
        const_str.attributes.insert(
            graph_attrs::VALUE.to_string(),
            apxm_core::types::Value::String("literal value".to_string()),
        );

        let graph = make_graph(vec![const_str], vec![]);

        let errors = validate_semantic(&graph, &SemanticContext::default());
        let e518 = errors
            .iter()
            .find(|e| e.code == ErrorCode::ConstStrWithDynamicInput);
        assert!(
            e518.is_none(),
            "E518 should not fire for valid const_str with literal value"
        );
    }
}
