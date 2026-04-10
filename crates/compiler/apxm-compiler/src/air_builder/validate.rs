use crate::{AirError, AirModule};
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::types::{get_operation_spec, provider_spec};
use std::collections::{HashMap, HashSet, VecDeque};

pub fn validate_module(module: &AirModule) -> Result<(), AirError> {
    if module.name.trim().is_empty() {
        return Err(AirError::Validation(
            "module name cannot be empty".to_string(),
        ));
    }

    if module.nodes.is_empty() {
        return Err(AirError::Validation(
            "module must contain at least one node".to_string(),
        ));
    }

    let mut node_ids = HashSet::with_capacity(module.nodes.len());
    for node in &module.nodes {
        if !node_ids.insert(node.id) {
            return Err(AirError::Validation(format!(
                "duplicate node id detected: {}",
                node.id
            )));
        }

        if node.name.trim().is_empty() {
            return Err(AirError::Validation(format!(
                "node {} has empty name",
                node.id
            )));
        }
    }

    for edge in &module.edges {
        if !node_ids.contains(&edge.from) {
            return Err(AirError::Validation(format!(
                "edge references unknown source node {}",
                edge.from
            )));
        }
        if !node_ids.contains(&edge.to) {
            return Err(AirError::Validation(format!(
                "edge references unknown destination node {}",
                edge.to
            )));
        }
    }

    validate_acyclic(module)?;
    validate_parameters(module)?;
    validate_providers(module)?;
    validate_token_space(module)?;
    validate_agent_references(module)?;
    validate_required_attributes(module)?;
    validate_node_refs(module)?;

    Ok(())
}

fn validate_acyclic(module: &AirModule) -> Result<(), AirError> {
    let mut in_degree: HashMap<u64, usize> =
        module.nodes.iter().map(|node| (node.id, 0)).collect();
    let mut adjacency: HashMap<u64, Vec<u64>> = HashMap::new();

    for edge in &module.edges {
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

    if visited != module.nodes.len() {
        return Err(AirError::Validation(
            "module must be acyclic (DAG validation failed)".to_string(),
        ));
    }

    Ok(())
}

fn validate_parameters(module: &AirModule) -> Result<(), AirError> {
    let mut names = HashSet::new();
    for parameter in &module.parameters {
        if parameter.name.trim().is_empty() {
            return Err(AirError::Validation(
                "parameter name cannot be empty".to_string(),
            ));
        }
        if parameter.type_name.trim().is_empty() {
            return Err(AirError::Validation(format!(
                "parameter '{}' has empty type",
                parameter.name
            )));
        }
        if !names.insert(parameter.name.clone()) {
            return Err(AirError::Validation(format!(
                "duplicate parameter name '{}'",
                parameter.name
            )));
        }
    }
    Ok(())
}

fn validate_token_space(module: &AirModule) -> Result<(), AirError> {
    let edge_count = module.edges.len() as u64;
    if edge_count == 0 {
        return Ok(());
    }

    let mut sorted_ids: Vec<u64> = module.nodes.iter().map(|n| n.id).collect();
    sorted_ids.sort_unstable();

    let is_sequential = sorted_ids
        .iter()
        .enumerate()
        .all(|(i, &id)| id == i as u64 + 1);

    if is_sequential {
        return Ok(());
    }

    let colliding: Vec<u64> = sorted_ids
        .iter()
        .copied()
        .filter(|&id| id >= 1 && id <= edge_count)
        .collect();

    if !colliding.is_empty() {
        return Err(AirError::Validation(format!(
            "node ID(s) {:?} collide with edge token IDs [1..{}]. \
             Use sequential node IDs starting at 1, or assign all \
             node IDs above {} (the edge count).",
            colliding, edge_count, edge_count
        )));
    }

    Ok(())
}

fn validate_providers(module: &AirModule) -> Result<(), AirError> {
    for node in &module.nodes {
        if !get_operation_spec(node.op).category.requires_llm() {
            continue;
        }

        let provider_value = match node.attributes.get(graph_attrs::PROVIDER) {
            Some(v) => v,
            None => continue,
        };

        let provider_name = match provider_value.as_str() {
            Some(s) => s,
            None => {
                return Err(AirError::Validation(format!(
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
            return Err(AirError::Validation(format!(
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

fn validate_agent_references(module: &AirModule) -> Result<(), AirError> {
    use apxm_core::types::operations::AISOperationType;

    let mut spawned: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
    for node in &module.nodes {
        if node.op != AISOperationType::SpawnAgent {
            continue;
        }
        let name = match node.attributes.get("agent_name").and_then(|v| v.as_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };
        if let Some(prev_id) = spawned.insert(name.clone(), node.id) {
            return Err(AirError::Validation(format!(
                "duplicate SPAWN_AGENT agent_name '{}': nodes {} and {} both declare it.",
                name, prev_id, node.id
            )));
        }
    }

    for node in &module.nodes {
        if node.op != AISOperationType::Communicate {
            continue;
        }
        let recipient = match node.attributes.get("recipient").and_then(|v| v.as_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };
        if !spawned.contains_key(&recipient) {
            let available: Vec<&str> = spawned.keys().map(|s| s.as_str()).collect();
            let hint = if available.is_empty() {
                "No SPAWN_AGENT nodes found in this module.".to_string()
            } else {
                format!("Spawned agents: [{}].", available.join(", "))
            };
            return Err(AirError::Validation(format!(
                "node '{}' (id={}, op=COMMUNICATE) references recipient '{}' \
                 which is not spawned by any SPAWN_AGENT. {}",
                node.name, node.id, recipient, hint
            )));
        }
    }

    Ok(())
}

fn validate_required_attributes(module: &AirModule) -> Result<(), AirError> {
    use apxm_core::types::operations::AISOperationType;

    for node in &module.nodes {
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
            AISOperationType::Merge => None,
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
            return Err(AirError::Validation(format!(
                "node '{}' (id={}, op={}) is missing required attribute '{}'.",
                node.name, node.id, node.op, attr
            )));
        }
    }

    Ok(())
}

fn validate_node_refs(module: &AirModule) -> Result<(), AirError> {
    use apxm_core::types::operations::AISOperationType;

    let node_ids: HashSet<u64> = module.nodes.iter().map(|n| n.id).collect();

    fn extract_node_refs(s: &str) -> Vec<u64> {
        let mut refs = Vec::new();
        let mut rest = s;
        while let Some(start) = rest.find("{{node_") {
            rest = &rest[start + 7..];
            if let Some(end) = rest.find("}}") {
                if let Ok(id) = rest[..end].trim().parse::<u64>() {
                    refs.push(id);
                }
                rest = &rest[end + 2..];
            } else {
                break;
            }
        }
        refs
    }

    for node in &module.nodes {
        if node.op == AISOperationType::Merge {
            if let Some(tokens_val) = node.attributes.get("tokens") {
                let tokens_str = serde_json::to_string(tokens_val).unwrap_or_default();
                for ref_id in extract_node_refs(&tokens_str) {
                    if !node_ids.contains(&ref_id) {
                        return Err(AirError::Validation(format!(
                            "node '{}' (id={}, op=MERGE) tokens references \
                             {{{{node_{}}}}} but no node with id {} exists.",
                            node.name, node.id, ref_id, ref_id
                        )));
                    }
                }
            }
        }

        if node.op == AISOperationType::Print {
            if let Some(msg_val) = node.attributes.get("message") {
                let msg_str = msg_val.as_str().unwrap_or("").to_string();
                for ref_id in extract_node_refs(&msg_str) {
                    if !node_ids.contains(&ref_id) {
                        return Err(AirError::Validation(format!(
                            "node '{}' (id={}, op=PRINT) message references \
                             {{{{node_{}}}}} but no node with id {} exists.",
                            node.name, node.id, ref_id, ref_id
                        )));
                    }
                }
            }
        }
    }

    Ok(())
}
