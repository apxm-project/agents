//! Prompt-channel facts derived from an emitted execution node.

use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::constants::graph::attrs::PromptInputRole;
use apxm_core::types::Value;
use apxm_core::types::compiler::PromptContractSummary;
use apxm_core::types::execution::Node;

/// Derive the role and tokenizer facts that remain valid in the artifact.
pub(super) fn summarize(node: &Node) -> PromptContractSummary {
    let input_roles = input_roles(node);
    PromptContractSummary {
        protected_input_count: input_roles
            .iter()
            .filter(|role| role.is_protected())
            .count(),
        input_roles,
        tokenizer: node
            .get_attribute(graph_attrs::MODEL)
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        shared_prefix_est_tokens: node
            .get_attribute(graph_attrs::SHARED_PREFIX_EST_TOKENS)
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok()),
    }
}

fn input_roles(node: &Node) -> Vec<PromptInputRole> {
    let input_count = node.input_tokens.len();
    let names: Vec<&str> = node
        .get_attribute(graph_attrs::INPUT_NAMES)
        .and_then(string_array)
        .unwrap_or_default();
    let explicit_roles = node
        .get_attribute(graph_attrs::INPUT_ROLES)
        .and_then(string_array);

    match explicit_roles {
        Some(roles) if roles.len() == input_count => roles
            .into_iter()
            .map(|role| PromptInputRole::parse(role).unwrap_or(PromptInputRole::Control))
            .collect(),
        _ => (0..input_count)
            .map(|index| {
                names.get(index).map_or(PromptInputRole::User, |name| {
                    PromptInputRole::from_legacy_input_name(name)
                })
            })
            .collect(),
    }
}

fn string_array(value: &Value) -> Option<Vec<&str>> {
    match value {
        Value::Array(values) => values.iter().map(Value::as_str).collect(),
        _ => None,
    }
}
