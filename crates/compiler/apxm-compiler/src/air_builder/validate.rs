use crate::template::{is_numeric_placeholder, parse_placeholder_names};
use crate::{AirError, AirModule};
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::types::{DependencyType, Value, get_operation_spec, provider_spec};
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
    validate_llm_operation_attributes(module)?;
    validate_template_placeholders(module)?;
    validate_node_refs(module)?;

    Ok(())
}

fn validate_acyclic(module: &AirModule) -> Result<(), AirError> {
    let mut in_degree: HashMap<u64, usize> = module.nodes.iter().map(|node| (node.id, 0)).collect();
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
        let name = match node
            .attributes
            .get(graph_attrs::AGENT_NAME)
            .and_then(|v| v.as_str())
        {
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
        let recipient = match node
            .attributes
            .get(graph_attrs::RECIPIENT)
            .and_then(|v| v.as_str())
        {
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
        let spawn_id = spawned
            .get(&recipient)
            .copied()
            .expect("recipient existence checked above");
        if !has_structural_spawn_dependency(module, spawn_id, node) {
            let has_control_edge = module.edges.iter().any(|edge| {
                edge.from == spawn_id
                    && edge.to == node.id
                    && matches!(edge.dependency, DependencyType::Control)
            });
            let hint = if has_control_edge {
                "A Control edge was found, but Control edges do not carry the \
                 spawn token into AIR operands."
            } else {
                "No Data dependency path from the matching SPAWN_AGENT was found."
            };
            return Err(AirError::Validation(format!(
                "node '{}' (id={}, op={}) targets spawned agent '{}' but does \
                 not depend on its SPAWN_AGENT token. {} Use the agent handle \
                 API or add a Data dependency from the spawn chain.",
                node.name, node.id, node.op, recipient, hint
            )));
        }
    }

    Ok(())
}

fn has_structural_spawn_dependency(
    module: &AirModule,
    spawn_id: u64,
    node: &crate::air_builder::AirNode,
) -> bool {
    let data_sources = data_input_sources(module, node.id);
    let message_input_count = collect_input_names(node).len();
    data_sources
        .iter()
        .skip(message_input_count)
        .any(|source_id| has_data_path(module, spawn_id, *source_id))
}

fn has_data_path(module: &AirModule, from: u64, to: u64) -> bool {
    if from == to {
        return true;
    }

    let mut adjacency: HashMap<u64, Vec<u64>> = HashMap::new();
    for edge in &module.edges {
        if matches!(edge.dependency, DependencyType::Data) {
            adjacency.entry(edge.from).or_default().push(edge.to);
        }
    }

    let mut visited = HashSet::new();
    let mut queue = VecDeque::from([from]);
    while let Some(current) = queue.pop_front() {
        if !visited.insert(current) {
            continue;
        }
        for next in adjacency.get(&current).into_iter().flatten().copied() {
            if next == to {
                return true;
            }
            queue.push_back(next);
        }
    }

    false
}

fn validate_required_attributes(module: &AirModule) -> Result<(), AirError> {
    use apxm_core::types::operations::AISOperationType;

    for node in &module.nodes {
        let required_attr: Option<&'static str> = match node.op {
            AISOperationType::SpawnAgent => Some(graph_attrs::AGENT_NAME),
            AISOperationType::Communicate => Some(graph_attrs::RECIPIENT),
            AISOperationType::ConstStr => Some(graph_attrs::VALUE),
            AISOperationType::InvTool => Some(graph_attrs::CAPABILITY),
            AISOperationType::Merge => None,
            _ => None,
        };
        let missing = required_attr.filter(|key| !node.attributes.contains_key(*key));

        if let Some(attr) = missing {
            return Err(AirError::Validation(format!(
                "node '{}' (id={}, op={}) is missing required attribute '{}'.",
                node.name, node.id, node.op, attr
            )));
        }
    }

    Ok(())
}

fn validate_llm_operation_attributes(module: &AirModule) -> Result<(), AirError> {
    use apxm_core::types::operations::AISOperationType;

    for node in &module.nodes {
        let Some(value) = node.attributes.get(graph_attrs::LLM_OPERATION) else {
            continue;
        };
        let Some(operation_name) = value.as_str() else {
            return Err(AirError::Validation(format!(
                "node '{}' (id={}, op={}) has non-string '{}' attribute.",
                node.name,
                node.id,
                node.op,
                graph_attrs::LLM_OPERATION
            )));
        };
        let Ok(operation) = operation_name.parse::<AISOperationType>() else {
            return Err(AirError::Validation(format!(
                "node '{}' (id={}, op={}) has invalid '{}' value '{}'.",
                node.name,
                node.id,
                node.op,
                graph_attrs::LLM_OPERATION,
                operation_name
            )));
        };
        if !matches!(
            operation,
            AISOperationType::Ask | AISOperationType::Think | AISOperationType::Reason
        ) {
            return Err(AirError::Validation(format!(
                "node '{}' (id={}, op={}) has '{}' value '{}', but only {}, {}, or {} \
                 are valid LLM turn operations.",
                node.name,
                node.id,
                node.op,
                graph_attrs::LLM_OPERATION,
                operation,
                AISOperationType::Ask,
                AISOperationType::Think,
                AISOperationType::Reason
            )));
        }
    }

    Ok(())
}

/// Validate that every `{name}` placeholder in template-bearing attributes
/// resolves to either an `input_names` entry on the node or a declared
/// module parameter, and that `input_names` matches the number of incoming
/// Data edges.
///
/// The canonical list of template-bearing attribute keys lives in
/// `apxm_core::constants::graph::attrs::TEMPLATE_BEARING_ATTRS` — this validator is driven
/// by that constant, so adding a new templated attribute does not require
/// touching the validator.
fn validate_template_placeholders(module: &AirModule) -> Result<(), AirError> {
    // Module-level parameter names — every `{name}` may resolve here. O(1) lookup.
    let param_names: HashSet<&str> = module.parameters.iter().map(|p| p.name.as_str()).collect();

    // Single sweep over edges: count incoming Data edges per destination node.
    let mut data_in_count: HashMap<u64, usize> = HashMap::with_capacity(module.nodes.len());
    for edge in &module.edges {
        if matches!(edge.dependency, DependencyType::Data) {
            *data_in_count.entry(edge.to).or_insert(0) += 1;
        }
    }

    // Single sweep over template-bearing attribute names → O(1) membership set
    // for the per-node attribute scan below.
    let template_attrs: HashSet<&str> = graph_attrs::TEMPLATE_BEARING_ATTRS
        .iter()
        .copied()
        .collect();

    for node in &module.nodes {
        let input_names = collect_input_names(node);
        let structural_data_count = structural_data_input_count(module, node);
        let in_count = data_in_count
            .get(&node.id)
            .copied()
            .unwrap_or(0)
            .saturating_sub(structural_data_count);

        // Length sanity: input_names mirrors the incoming Data edges.
        if !input_names.is_empty() && input_names.len() != in_count {
            return Err(AirError::Validation(format!(
                "node '{}' (id={}, op={}): input_names has {} entries but \
                 the node has {} incoming Data edge(s).",
                node.name,
                node.id,
                node.op,
                input_names.len(),
                in_count
            )));
        }

        let input_set: HashSet<&str> = input_names.iter().copied().collect();

        // Walk the node's attributes once; only validate keys that are in
        // the template-bearing set. This avoids a nested loop over every
        // template attribute name on every node.
        for (attr_key, attr_val) in &node.attributes {
            if !template_attrs.contains(attr_key.as_str()) {
                continue;
            }
            let Value::String(template) = attr_val else {
                continue;
            };
            check_template_placeholders(template, attr_key, node, &input_set, &param_names)?;
        }
    }

    Ok(())
}

fn structural_data_input_count(module: &AirModule, node: &crate::air_builder::AirNode) -> usize {
    use apxm_core::types::operations::AISOperationType;

    if node.op != AISOperationType::Communicate {
        return 0;
    }

    let data_source_count = data_input_sources(module, node.id).len();
    data_source_count.saturating_sub(collect_input_names(node).len())
}

fn data_input_sources(module: &AirModule, node_id: u64) -> Vec<u64> {
    module
        .edges
        .iter()
        .filter(|edge| edge.to == node_id && matches!(edge.dependency, DependencyType::Data))
        .map(|edge| edge.from)
        .collect()
}

/// Extract a node's `input_names` parallel array as borrowed string slices.
/// Returns an empty vector when the attribute is absent or malformed.
fn collect_input_names(node: &crate::air_builder::AirNode) -> Vec<&str> {
    match node.attributes.get(graph_attrs::INPUT_NAMES) {
        Some(Value::Array(arr)) => arr
            .iter()
            .filter_map(|v| match v {
                Value::String(s) => Some(s.as_str()),
                _ => None,
            })
            .collect(),
        Some(Value::String(s)) => vec![s.as_str()],
        _ => Vec::new(),
    }
}

/// Validate every placeholder in `template`, producing a precise diagnostic
/// on the first failure.
fn check_template_placeholders(
    template: &str,
    attr_key: &str,
    node: &crate::air_builder::AirNode,
    input_set: &HashSet<&str>,
    param_names: &HashSet<&str>,
) -> Result<(), AirError> {
    for placeholder in parse_placeholder_names(template) {
        if is_numeric_placeholder(placeholder) {
            return Err(AirError::Validation(format!(
                "node '{}' (id={}, op={}, attr={}): numeric placeholder \
                 '{{{}}}' is not allowed in templates; use a named reference \
                 like '{{<source_name>}}' that matches an entry in \
                 input_names or a declared module parameter.",
                node.name, node.id, node.op, attr_key, placeholder
            )));
        }
        if input_set.contains(placeholder) || param_names.contains(placeholder) {
            continue;
        }
        let suggestion = nearest_name(placeholder, input_set, param_names);
        let hint = match suggestion {
            Some(s) => format!(" (did you mean '{{{}}}'?)", s),
            None => String::new(),
        };
        let known = build_known_names_hint(input_set, param_names);
        return Err(AirError::Validation(format!(
            "node '{}' (id={}, op={}, attr={}): placeholder '{{{}}}' \
             references no known input or parameter{}. {}",
            node.name, node.id, node.op, attr_key, placeholder, hint, known
        )));
    }
    Ok(())
}

/// Find the name in `inputs ∪ params` whose Levenshtein distance to
/// `target` is at most 1. Returns `None` if no such name exists.
fn nearest_name<'a>(
    target: &str,
    inputs: &'a HashSet<&str>,
    params: &'a HashSet<&str>,
) -> Option<&'a str> {
    inputs
        .iter()
        .chain(params.iter())
        .copied()
        .find(|cand| levenshtein_at_most_1(target, cand))
}

fn build_known_names_hint(inputs: &HashSet<&str>, params: &HashSet<&str>) -> String {
    let mut all: Vec<&str> = inputs.iter().chain(params.iter()).copied().collect();
    all.sort_unstable();
    all.dedup();
    if all.is_empty() {
        "No input_names or module parameters are in scope.".to_string()
    } else {
        format!("Known names: [{}].", all.join(", "))
    }
}

/// Returns true when the Levenshtein distance between `a` and `b` is ≤ 1.
fn levenshtein_at_most_1(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    let ab: Vec<char> = a.chars().collect();
    let bb: Vec<char> = b.chars().collect();
    let (la, lb) = (ab.len(), bb.len());
    if la.abs_diff(lb) > 1 {
        return false;
    }
    if la == lb {
        // Exactly one substitution.
        let diffs = ab.iter().zip(bb.iter()).filter(|(x, y)| x != y).count();
        return diffs == 1;
    }
    // Exactly one insertion or deletion. Walk both strings allowing one skip.
    let (short, long) = if la < lb { (&ab, &bb) } else { (&bb, &ab) };
    let mut i = 0;
    let mut j = 0;
    let mut skipped = false;
    while i < short.len() && j < long.len() {
        if short[i] == long[j] {
            i += 1;
            j += 1;
        } else if !skipped {
            skipped = true;
            j += 1;
        } else {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod agent_reference_validation_tests {
    use super::*;
    use crate::air_builder::{AirEdge, AirNode};
    use apxm_core::types::AISOperationType;
    use std::collections::HashMap;

    fn spawn_and_communicate_module(edges: Vec<AirEdge>) -> AirModule {
        AirModule {
            name: "agent_refs".into(),
            nodes: vec![
                AirNode {
                    id: 1,
                    name: "spawn_worker".into(),
                    op: AISOperationType::SpawnAgent,
                    attributes: HashMap::from([(
                        graph_attrs::AGENT_NAME.into(),
                        Value::String("worker".into()),
                    )]),
                },
                AirNode {
                    id: 2,
                    name: "send_worker".into(),
                    op: AISOperationType::Communicate,
                    attributes: HashMap::from([(
                        graph_attrs::RECIPIENT.into(),
                        Value::String("worker".into()),
                    )]),
                },
            ],
            edges,
            parameters: Vec::new(),
            metadata: HashMap::new(),
        }
    }

    #[test]
    fn accepts_communicate_with_spawn_data_path() {
        let module = spawn_and_communicate_module(vec![AirEdge {
            from: 1,
            to: 2,
            dependency: DependencyType::Data,
        }]);

        validate_agent_references(&module).expect("data path should satisfy spawn dependency");
    }

    #[test]
    fn rejects_communicate_without_spawn_data_path() {
        let module = spawn_and_communicate_module(Vec::new());

        let err = validate_agent_references(&module).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("does not depend on its SPAWN_AGENT token"),
            "unexpected diagnostic: {msg}"
        );
    }

    #[test]
    fn rejects_control_edge_from_spawn_to_communicate() {
        let module = spawn_and_communicate_module(vec![AirEdge {
            from: 1,
            to: 2,
            dependency: DependencyType::Control,
        }]);

        let err = validate_agent_references(&module).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("Control edges do not carry the spawn token"),
            "unexpected diagnostic: {msg}"
        );
    }

    #[test]
    fn accepts_communicate_llm_turn_operation() {
        let mut module = spawn_and_communicate_module(vec![AirEdge {
            from: 1,
            to: 2,
            dependency: DependencyType::Data,
        }]);
        module.nodes[1].attributes.insert(
            graph_attrs::LLM_OPERATION.into(),
            Value::String(AISOperationType::Reason.to_string()),
        );

        validate_module(&module).expect("REASON is a valid LLM turn operation");
    }

    #[test]
    fn rejects_non_llm_turn_operation_attribute() {
        let mut module = spawn_and_communicate_module(vec![AirEdge {
            from: 1,
            to: 2,
            dependency: DependencyType::Data,
        }]);
        module.nodes[1].attributes.insert(
            graph_attrs::LLM_OPERATION.into(),
            Value::String(AISOperationType::Plan.to_string()),
        );

        let err = validate_module(&module).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("valid LLM turn operations"),
            "unexpected diagnostic: {msg}"
        );
    }
}

#[cfg(test)]
mod template_validation_tests {
    use super::*;
    use crate::air_builder::{AirEdge, AirNode, AirParam};
    use apxm_core::types::AISOperationType;
    use std::collections::HashMap;

    /// Build a two-node module: a producer (ConstStr `value`) feeds an Ask
    /// node whose template attribute is `template`. `input_names` is set on
    /// the consumer to `[input_name]`. Module parameters are `params`.
    fn ask_after_const_module(
        template: &str,
        input_name: Option<&str>,
        params: &[&str],
    ) -> AirModule {
        let mut consumer_attrs = HashMap::new();
        consumer_attrs.insert(
            graph_attrs::TEMPLATE_STR.into(),
            Value::String(template.into()),
        );
        if let Some(name) = input_name {
            consumer_attrs.insert(
                graph_attrs::INPUT_NAMES.into(),
                Value::Array(vec![Value::String(name.into())]),
            );
        }
        AirModule {
            name: "tpl_test".into(),
            nodes: vec![
                AirNode {
                    id: 1,
                    name: "seed".into(),
                    op: AISOperationType::ConstStr,
                    attributes: HashMap::from([(
                        graph_attrs::VALUE.into(),
                        Value::String("hello".into()),
                    )]),
                },
                AirNode {
                    id: 2,
                    name: "consumer".into(),
                    op: AISOperationType::Ask,
                    attributes: consumer_attrs,
                },
            ],
            edges: vec![AirEdge {
                from: 1,
                to: 2,
                dependency: DependencyType::Data,
            }],
            parameters: params
                .iter()
                .map(|n| AirParam {
                    name: (*n).to_string(),
                    type_name: "str".into(),
                })
                .collect(),
            metadata: HashMap::new(),
        }
    }

    #[test]
    fn accepts_named_placeholder_matching_input_names() {
        let module = ask_after_const_module("Hello {seed}", Some("seed"), &[]);
        validate_template_placeholders(&module).expect("should pass");
    }

    #[test]
    fn accepts_named_placeholder_matching_module_parameter() {
        // No incoming Data edge → drop the edge for a clean param-only test.
        let mut module = ask_after_const_module("Process {topic}", None, &["topic"]);
        module.edges.clear();
        module.nodes.remove(0); // drop ConstStr; leaves the consumer alone
        validate_template_placeholders(&module).expect("module-parameter ref should pass");
    }

    #[test]
    fn rejects_legacy_node_reference_syntax() {
        let module = ask_after_const_module("Hello {{node_1}}", None, &[]);
        let err = validate_module(&module).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("legacy node placeholder syntax")
                || msg.contains("references no known input or parameter"),
            "unexpected diagnostic: {msg}"
        );
    }

    #[test]
    fn rejects_numeric_placeholder() {
        let module = ask_after_const_module("Hello {0}", Some("seed"), &[]);
        let err = validate_template_placeholders(&module).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("numeric placeholder"),
            "unexpected diagnostic: {msg}"
        );
        assert!(
            msg.contains("{0}"),
            "diagnostic should quote the offender: {msg}"
        );
    }

    #[test]
    fn rejects_unknown_placeholder_with_known_names_hint() {
        let module = ask_after_const_module("Hello {nope}", Some("seed"), &[]);
        let err = validate_template_placeholders(&module).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("references no known input or parameter"));
        assert!(msg.contains("seed"), "should list 'seed' as known: {msg}");
    }

    #[test]
    fn suggests_levenshtein_one_neighbor() {
        let module = ask_after_const_module("Hello {seedz}", Some("seed"), &[]);
        let err = validate_template_placeholders(&module).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("did you mean '{seed}'?"),
            "missing suggestion in: {msg}"
        );
    }

    #[test]
    fn rejects_input_names_length_mismatch() {
        // input_names has 2 entries but only 1 incoming Data edge.
        let mut module = ask_after_const_module("Hello {a}", None, &[]);
        module
            .nodes
            .iter_mut()
            .find(|n| n.id == 2)
            .unwrap()
            .attributes
            .insert(
                graph_attrs::INPUT_NAMES.into(),
                Value::Array(vec![Value::String("a".into()), Value::String("b".into())]),
            );
        let err = validate_template_placeholders(&module).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("input_names has 2 entries but the node has 1"),
            "unexpected diagnostic: {msg}"
        );
    }

    #[test]
    fn communicate_input_names_ignore_direct_spawn_token_edge() {
        let module = AirModule {
            name: "communicate_template".into(),
            nodes: vec![
                AirNode {
                    id: 1,
                    name: "spawn_worker".into(),
                    op: AISOperationType::SpawnAgent,
                    attributes: HashMap::from([(
                        graph_attrs::AGENT_NAME.into(),
                        Value::String("worker".into()),
                    )]),
                },
                AirNode {
                    id: 2,
                    name: "task".into(),
                    op: AISOperationType::ConstStr,
                    attributes: HashMap::from([(
                        graph_attrs::VALUE.into(),
                        Value::String("hello".into()),
                    )]),
                },
                AirNode {
                    id: 3,
                    name: "send_worker".into(),
                    op: AISOperationType::Communicate,
                    attributes: HashMap::from([
                        (
                            graph_attrs::RECIPIENT.into(),
                            Value::String("worker".into()),
                        ),
                        (
                            graph_attrs::MESSAGE.into(),
                            Value::String("Process {task}".into()),
                        ),
                        (
                            graph_attrs::INPUT_NAMES.into(),
                            Value::Array(vec![Value::String("task".into())]),
                        ),
                    ]),
                },
            ],
            edges: vec![
                AirEdge {
                    from: 2,
                    to: 3,
                    dependency: DependencyType::Data,
                },
                AirEdge {
                    from: 1,
                    to: 3,
                    dependency: DependencyType::Data,
                },
            ],
            parameters: Vec::new(),
            metadata: HashMap::new(),
        };

        validate_module(&module).expect("spawn token edge is structural");
    }

    #[test]
    fn ignores_non_template_string_attrs() {
        // `agent_name` is not a template-bearing attribute; "{0}" inside it
        // must NOT trip the validator.
        let mut module = ask_after_const_module("Hello {seed}", Some("seed"), &[]);
        module
            .nodes
            .iter_mut()
            .find(|n| n.id == 2)
            .unwrap()
            .attributes
            .insert(
                graph_attrs::AGENT_NAME.into(),
                Value::String("agent_{0}".into()),
            );
        validate_template_placeholders(&module).expect("non-template attrs are ignored");
    }

    #[test]
    fn levenshtein_at_most_1_handles_substitution_insertion_deletion() {
        assert!(levenshtein_at_most_1("seed", "seed"));
        assert!(levenshtein_at_most_1("seed", "feed")); // 1 substitution
        assert!(levenshtein_at_most_1("seed", "seeds")); // 1 insertion
        assert!(levenshtein_at_most_1("seeds", "seed")); // 1 deletion
        assert!(!levenshtein_at_most_1("seed", "seedz1")); // distance 2
        assert!(!levenshtein_at_most_1("abc", "xyz")); // distance 3
    }
}

fn validate_node_refs(module: &AirModule) -> Result<(), AirError> {
    for node in &module.nodes {
        for (attr_key, attr_value) in &node.attributes {
            if let Some(placeholder) = find_legacy_node_placeholder(attr_value) {
                return Err(AirError::Validation(format!(
                    "node '{}' (id={}, op={}, attr={}): legacy node placeholder syntax '{}' \
                     is not supported. Use named placeholders like '{{source}}' with \
                     input_names for template-bearing string attributes, and use Data edges \
                     for structural inputs instead of node-id references.",
                    node.name, node.id, node.op, attr_key, placeholder
                )));
            }
        }
    }

    Ok(())
}

fn find_legacy_node_placeholder(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => find_legacy_node_placeholder_in_str(text),
        Value::Array(items) => items.iter().find_map(find_legacy_node_placeholder),
        Value::Object(map) => map.values().find_map(find_legacy_node_placeholder),
        _ => None,
    }
}

fn find_legacy_node_placeholder_in_str(text: &str) -> Option<String> {
    let mut rest = text;
    while let Some(start) = rest.find("{{node_") {
        let candidate = &rest[start..];
        let Some(end) = candidate.find("}}") else {
            break;
        };
        let placeholder = &candidate[..end + 2];
        let inner = &placeholder["{{node_".len()..placeholder.len() - 2];
        if inner.chars().all(|ch| ch.is_ascii_digit()) {
            return Some(placeholder.to_string());
        }
        rest = &candidate[end + 2..];
    }
    None
}
