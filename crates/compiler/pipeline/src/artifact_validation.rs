use std::collections::HashSet;

use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::types::{AISOperationType, ExecutionDag, Node, Value};

use crate::template::{is_numeric_placeholder, parse_placeholder_names, placeholder_root};

pub(crate) fn validate_template_placeholders(dags: &[ExecutionDag]) -> Result<(), String> {
    for dag in dags {
        validate_dag_template_placeholders(dag)?;
    }
    Ok(())
}

fn validate_dag_template_placeholders(dag: &ExecutionDag) -> Result<(), String> {
    let param_names: HashSet<&str> = dag
        .metadata
        .parameters
        .iter()
        .map(|param| param.name.as_str())
        .collect();
    let template_attrs: HashSet<&str> = graph_attrs::TEMPLATE_BEARING_ATTRS
        .iter()
        .copied()
        .chain([graph_attrs::PROMPT])
        .collect();

    for node in &dag.nodes {
        let input_names = collect_input_names(node);
        check_required_llm_prompt_contract(node)?;
        if input_names.len() > node.input_tokens.len() {
            return Err(format!(
                "node {} (op={}): input_names has {} entries but the node has {} input token(s)",
                node.id,
                node.op_type,
                input_names.len(),
                node.input_tokens.len()
            ));
        }

        let input_set: HashSet<&str> = input_names.iter().copied().collect();
        for (attr_key, attr_value) in &node.attributes {
            if !template_attrs.contains(attr_key.as_str()) {
                continue;
            }
            let Some(template) = attr_value.as_str() else {
                continue;
            };
            check_template_placeholders(template, attr_key, node, &input_set, &param_names)?;
        }
    }

    Ok(())
}

fn check_required_llm_prompt_contract(node: &Node) -> Result<(), String> {
    if !matches!(
        node.op_type,
        AISOperationType::Ask | AISOperationType::Think | AISOperationType::Reason
    ) {
        return Ok(());
    }

    let template = node
        .attributes
        .get(graph_attrs::TEMPLATE_STR)
        .or_else(|| node.attributes.get(graph_attrs::PROMPT))
        .and_then(Value::as_str)
        .unwrap_or("");

    if !node.input_tokens.is_empty() && template.trim().is_empty() {
        return Err(format!(
            "node {} (op={}): LLM node has {} input token(s) but an empty prompt template; run build-prompt or provide a template_str/input_names contract",
            node.id,
            node.op_type,
            node.input_tokens.len()
        ));
    }

    let input_names = collect_input_names(node);
    if !node.input_tokens.is_empty() && input_names.len() != node.input_tokens.len() {
        return Err(format!(
            "node {} (op={}): LLM node has {} input token(s) but {} input_names entries; run build-prompt or provide a complete template_str/input_names contract",
            node.id,
            node.op_type,
            node.input_tokens.len(),
            input_names.len()
        ));
    }

    Ok(())
}

fn collect_input_names(node: &Node) -> Vec<&str> {
    match node.attributes.get(graph_attrs::INPUT_NAMES) {
        Some(Value::Array(values)) => values.iter().filter_map(Value::as_str).collect(),
        Some(Value::String(value)) => vec![value.as_str()],
        _ => Vec::new(),
    }
}

fn check_template_placeholders(
    template: &str,
    attr_key: &str,
    node: &Node,
    input_names: &HashSet<&str>,
    param_names: &HashSet<&str>,
) -> Result<(), String> {
    for placeholder in parse_placeholder_names(template) {
        let root = placeholder_root(placeholder);
        if is_numeric_placeholder(root) {
            return Err(format!(
                "node {} (op={}, attr={}): numeric placeholder '{{{}}}' is not allowed; use a named input or declared parameter",
                node.id, node.op_type, attr_key, placeholder
            ));
        }
        if input_names.contains(root) || param_names.contains(root) {
            continue;
        }
        return Err(format!(
            "node {} (op={}, attr={}): placeholder '{{{}}}' references no known input or parameter. {}",
            node.id,
            node.op_type,
            attr_key,
            placeholder,
            known_names(input_names, param_names)
        ));
    }
    Ok(())
}

fn known_names(input_names: &HashSet<&str>, param_names: &HashSet<&str>) -> String {
    let mut names: Vec<&str> = input_names
        .iter()
        .chain(param_names.iter())
        .copied()
        .collect();
    names.sort_unstable();
    names.dedup();
    if names.is_empty() {
        "No input_names or module parameters are in scope.".to_string()
    } else {
        format!("Known names: [{}].", names.join(", "))
    }
}
