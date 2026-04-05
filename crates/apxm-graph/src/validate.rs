use crate::{ApxmGraph, GraphError};
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::types::{get_operation_spec, provider_spec};
use std::collections::{HashMap, HashSet, VecDeque};

pub fn validate_graph(graph: &ApxmGraph) -> Result<(), GraphError> {
    if graph.name.trim().is_empty() {
        return Err(GraphError::Validation(
            "graph name cannot be empty".to_string(),
        ));
    }

    if graph.nodes.is_empty() {
        return Err(GraphError::Validation(
            "graph must contain at least one node".to_string(),
        ));
    }

    let mut node_ids = HashSet::with_capacity(graph.nodes.len());
    for node in &graph.nodes {
        if !node_ids.insert(node.id) {
            return Err(GraphError::Validation(format!(
                "duplicate node id detected: {}",
                node.id
            )));
        }

        if node.name.trim().is_empty() {
            return Err(GraphError::Validation(format!(
                "node {} has empty name",
                node.id
            )));
        }
    }

    for edge in &graph.edges {
        if !node_ids.contains(&edge.from) {
            return Err(GraphError::Validation(format!(
                "edge references unknown source node {}",
                edge.from
            )));
        }
        if !node_ids.contains(&edge.to) {
            return Err(GraphError::Validation(format!(
                "edge references unknown destination node {}",
                edge.to
            )));
        }
    }

    validate_acyclic(graph)?;
    validate_parameters(graph)?;
    validate_providers(graph)?;
    validate_token_space(graph)?;
    validate_agent_references(graph)?;
    validate_required_attributes(graph)?;

    Ok(())
}

fn validate_acyclic(graph: &ApxmGraph) -> Result<(), GraphError> {
    let mut in_degree: HashMap<u64, usize> = graph.nodes.iter().map(|node| (node.id, 0)).collect();
    let mut adjacency: HashMap<u64, Vec<u64>> = HashMap::new();

    for edge in &graph.edges {
        adjacency.entry(edge.from).or_default().push(edge.to);
        *in_degree.entry(edge.to).or_insert(0) += 1;
    }

    let mut queue: VecDeque<u64> = in_degree
        .iter()
        .filter_map(|(id, degree)| if *degree == 0 { Some(*id) } else { None })
        .collect();
    let mut visited = 0usize;

    while let Some(node_id) = queue.pop_front() {
        visited += 1;
        if let Some(neighbors) = adjacency.get(&node_id) {
            for neighbor in neighbors {
                if let Some(current) = in_degree.get_mut(neighbor) {
                    *current = current.saturating_sub(1);
                    if *current == 0 {
                        queue.push_back(*neighbor);
                    }
                }
            }
        }
    }

    if visited != graph.nodes.len() {
        return Err(GraphError::Validation(
            "graph must be acyclic (DAG validation failed)".to_string(),
        ));
    }

    Ok(())
}

fn validate_parameters(graph: &ApxmGraph) -> Result<(), GraphError> {
    let mut names = HashSet::new();
    for parameter in &graph.parameters {
        if parameter.name.trim().is_empty() {
            return Err(GraphError::Validation(
                "parameter name cannot be empty".to_string(),
            ));
        }
        if parameter.type_name.trim().is_empty() {
            return Err(GraphError::Validation(format!(
                "parameter '{}' has empty type",
                parameter.name
            )));
        }
        if !names.insert(parameter.name.clone()) {
            return Err(GraphError::Validation(format!(
                "duplicate parameter name '{}'",
                parameter.name
            )));
        }
    }
    Ok(())
}

/// Validate that node IDs do not collide with edge-assigned token IDs.
///
/// The runtime assigns token IDs 1..=edge_count to edges. Node IDs in that
/// range produce a "Duplicate producer" panic at runtime because the scheduler
/// sees both the node's synthetic output token AND the edge token as producers
/// for the same slot. This check catches the problem at graph load time.
///
/// Safe pattern: sequential node IDs starting at 1 — the runtime then assigns
/// each edge its own unique token and node N's primary output token equals N.
fn validate_token_space(graph: &ApxmGraph) -> Result<(), GraphError> {
    let edge_count = graph.edges.len() as u64;
    if edge_count == 0 {
        return Ok(());
    }

    let mut sorted_ids: Vec<u64> = graph.nodes.iter().map(|n| n.id).collect();
    sorted_ids.sort_unstable();

    // Sequential 1..=N is always safe: each node's primary output token equals
    // its own ID, which is exactly the edge token the lowering assigns.
    let is_sequential = sorted_ids
        .iter()
        .enumerate()
        .all(|(i, &id)| id == i as u64 + 1);

    if is_sequential {
        return Ok(());
    }

    // Non-sequential IDs: any node ID in [1, edge_count] is a potential conflict.
    let colliding: Vec<u64> = sorted_ids
        .iter()
        .copied()
        .filter(|&id| id >= 1 && id <= edge_count)
        .collect();

    if !colliding.is_empty() {
        return Err(GraphError::Validation(format!(
            "node ID(s) {:?} collide with edge token IDs [1..{}].              Token IDs are assigned 1..=edge_count per edge index, so              non-sequential node IDs must not fall in that range.              Fix: use sequential node IDs starting at 1, or assign all              node IDs above {} (the edge count).",
            colliding, edge_count, edge_count
        )));
    }

    Ok(())
}

/// Validate that LLM nodes reference registered providers.
fn validate_providers(graph: &ApxmGraph) -> Result<(), GraphError> {
    for node in &graph.nodes {
        if !get_operation_spec(node.op).category.requires_llm() {
            continue;
        }

        let provider_value = match node.attributes.get(graph_attrs::PROVIDER) {
            Some(v) => v,
            None => continue, // No explicit provider — runtime will use its default.
        };

        let provider_name = match provider_value.as_str() {
            Some(s) => s,
            None => {
                return Err(GraphError::Validation(format!(
                    "node '{}' ({}): provider attribute must be a string",
                    node.name, node.op
                )));
            }
        };

        if provider_spec::resolve_builtin_provider(provider_name).is_none() {
            let valid: Vec<&str> = provider_spec::BUILTIN_PROVIDERS
                .iter()
                .map(|s| s.id)
                .collect();
            return Err(GraphError::Validation(format!(
                "node '{}' ({}): unknown provider '{}'. Registered providers: {}",
                node.name,
                node.op,
                provider_name,
                valid.join(", ")
            )));
        }
    }

    Ok(())
}

/// Validate that every COMMUNICATE recipient is spawned by a SPAWN_AGENT
/// in the same graph, and that every SPAWN_AGENT has a unique agent_name.
///
/// Catches: typos in recipient names, missing SPAWN_AGENT nodes, and
/// duplicate agent names that would cause session collisions at runtime.
fn validate_agent_references(graph: &ApxmGraph) -> Result<(), GraphError> {
    use apxm_core::types::operations::AISOperationType;

    // Collect all agent names declared by SPAWN_AGENT nodes.
    let mut spawned: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
    for node in &graph.nodes {
        if node.op != AISOperationType::SpawnAgent {
            continue;
        }
        let name = match node.attributes.get("agent_name").and_then(|v| v.as_str()) {
            Some(s) => s.to_string(),
            None => continue, // Missing attribute caught by validate_required_attributes.
        };
        if let Some(prev_id) = spawned.insert(name.clone(), node.id) {
            return Err(GraphError::Validation(format!(
                "duplicate SPAWN_AGENT agent_name '{}': nodes {} and {} both declare it. \
                 Each agent must have a unique name within the graph.",
                name, prev_id, node.id
            )));
        }
    }

    // Every COMMUNICATE recipient must appear in spawned.
    for node in &graph.nodes {
        if node.op != AISOperationType::Communicate {
            continue;
        }
        let recipient = match node.attributes.get("recipient").and_then(|v| v.as_str()) {
            Some(s) => s.to_string(),
            None => continue, // Missing attribute caught by validate_required_attributes.
        };
        if !spawned.contains_key(&recipient) {
            let available: Vec<&str> = spawned.keys().map(|s| s.as_str()).collect();
            let hint = if available.is_empty() {
                "No SPAWN_AGENT nodes found in this graph.".to_string()
            } else {
                format!("Spawned agents in this graph: [{}].", available.join(", "))
            };
            return Err(GraphError::Validation(format!(
                "node '{}' (id={}, op=COMMUNICATE) references recipient '{}' \
                 which is not spawned by any SPAWN_AGENT in this graph. {}",
                node.name, node.id, recipient, hint
            )));
        }
    }

    Ok(())
}

/// Validate that required attributes are present on nodes that need them.
///
/// Catches missing attributes that would produce cryptic runtime errors
/// (KeyError, missing field panics, etc.) well before execution starts.
fn validate_required_attributes(graph: &ApxmGraph) -> Result<(), GraphError> {
    use apxm_core::types::operations::AISOperationType;

    for node in &graph.nodes {
        let missing: Option<&str> = match node.op {
            AISOperationType::SpawnAgent => {
                if !node.attributes.contains_key("agent_name") {
                    Some("agent_name")
                } else {
                    None
                }
            }
            AISOperationType::Communicate => {
                if !node.attributes.contains_key("recipient") {
                    Some("recipient")
                } else {
                    None
                }
            }
            AISOperationType::ConstStr => {
                if !node.attributes.contains_key("value") {
                    Some("value")
                } else {
                    None
                }
            }
            AISOperationType::Merge => {
                if !node.attributes.contains_key("tokens") {
                    Some("tokens")
                } else {
                    None
                }
            }
            AISOperationType::InvTool => {
                if !node.attributes.contains_key("capability") {
                    Some("capability")
                } else {
                    None
                }
            }
            _ => None,
        };

        if let Some(attr) = missing {
            return Err(GraphError::Validation(format!(
                "node '{}' (id={}, op={}) is missing required attribute '{}'.",
                node.name, node.id, node.op, attr
            )));
        }
    }

    Ok(())
}
