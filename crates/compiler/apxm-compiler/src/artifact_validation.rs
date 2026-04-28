use std::collections::HashSet;

use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::types::{AISOperationType, ExecutionDag, Node, Value};

use crate::template::{is_numeric_placeholder, parse_placeholder_names};

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
        if is_numeric_placeholder(placeholder) {
            return Err(format!(
                "node {} (op={}, attr={}): numeric placeholder '{{{}}}' is not allowed; use a named input or declared parameter",
                node.id, node.op_type, attr_key, placeholder
            ));
        }
        if input_names.contains(placeholder) || param_names.contains(placeholder) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_core::types::execution::FlowParameter;
    use apxm_core::types::{AISOperationType, Node};

    fn dag_with_node(node: Node, params: Vec<&str>) -> ExecutionDag {
        ExecutionDag {
            nodes: vec![node],
            edges: vec![],
            entry_nodes: vec![1],
            exit_nodes: vec![1],
            metadata: apxm_core::types::DagMetadata {
                name: Some("test".to_string()),
                is_entry: true,
                parameters: params
                    .into_iter()
                    .map(|name| FlowParameter {
                        name: name.to_string(),
                        type_name: "str".to_string(),
                    })
                    .collect(),
            },
        }
    }

    fn node_with_attr(attr: &str, value: &str) -> Node {
        let mut node = Node::new(1, AISOperationType::Communicate);
        node.attributes
            .insert(attr.to_string(), Value::String(value.to_string()));
        node
    }

    #[test]
    fn accepts_runtime_parameter_placeholders() {
        let node = node_with_attr(graph_attrs::MESSAGE, "Task:\n{{task}}");
        let dag = dag_with_node(node, vec!["task"]);

        validate_template_placeholders(&[dag]).expect("runtime parameter should validate");
    }

    #[test]
    fn rejects_unbound_runtime_parameter_placeholders() {
        let node = node_with_attr(graph_attrs::MESSAGE, "Task:\n{task}");
        let dag = dag_with_node(node, vec![]);

        let err = validate_template_placeholders(&[dag]).unwrap_err();
        assert!(
            err.contains("placeholder '{task}' references no known input or parameter"),
            "unexpected diagnostic: {err}"
        );
    }

    #[test]
    fn accepts_dataflow_input_placeholders() {
        let mut node = node_with_attr(graph_attrs::TEMPLATE_STR, "Review {report}");
        node.input_tokens = vec![10];
        node.attributes.insert(
            graph_attrs::INPUT_NAMES.to_string(),
            Value::Array(vec![Value::String("report".to_string())]),
        );
        let dag = dag_with_node(node, vec![]);

        validate_template_placeholders(&[dag]).expect("input_names placeholder should validate");
    }

    #[test]
    fn rejects_numeric_placeholders() {
        let node = node_with_attr(graph_attrs::MESSAGE, "Task: {0}");
        let dag = dag_with_node(node, vec![]);

        let err = validate_template_placeholders(&[dag]).unwrap_err();
        assert!(
            err.contains("numeric placeholder '{0}' is not allowed"),
            "unexpected diagnostic: {err}"
        );
    }

    #[test]
    fn rejects_input_name_without_input_token() {
        let mut node = node_with_attr(graph_attrs::TEMPLATE_STR, "Review {report}");
        node.attributes.insert(
            graph_attrs::INPUT_NAMES.to_string(),
            Value::Array(vec![Value::String("report".to_string())]),
        );
        let dag = dag_with_node(node, vec![]);

        let err = validate_template_placeholders(&[dag]).unwrap_err();
        assert!(
            err.contains("input_names has 1 entries but the node has 0 input token"),
            "unexpected diagnostic: {err}"
        );
    }

    #[test]
    fn rejects_context_only_llm_when_build_prompt_did_not_run() {
        let mut node = Node::new(1, AISOperationType::Ask);
        node.attributes.insert(
            graph_attrs::TEMPLATE_STR.to_string(),
            Value::String(String::new()),
        );
        node.input_tokens = vec![10];
        let dag = dag_with_node(node, vec![]);

        let err = validate_template_placeholders(&[dag]).unwrap_err();
        assert!(
            err.contains("empty prompt template"),
            "unexpected diagnostic: {err}"
        );
    }

    #[test]
    fn rejects_llm_context_without_input_names() {
        let mut node = Node::new(1, AISOperationType::Ask);
        node.attributes.insert(
            graph_attrs::TEMPLATE_STR.to_string(),
            Value::String("Summarize the upstream artifact".to_string()),
        );
        node.input_tokens = vec![10];
        let dag = dag_with_node(node, vec![]);

        let err = validate_template_placeholders(&[dag]).unwrap_err();
        assert!(
            err.contains("input_names entries"),
            "unexpected diagnostic: {err}"
        );
    }
}
