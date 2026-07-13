//! Prompt-channel facts derived from an emitted execution node.

use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::constants::graph::attrs::PromptInputRole;
use apxm_core::types::Value;
use apxm_core::types::compiler::PromptContractSummary;
use apxm_core::types::execution::Node;

use super::evidence::TokenizerEvidence;

/// Derive the role and tokenizer facts that remain valid in the artifact.
pub(super) fn summarize(
    node: &Node,
    tokenizer: Option<TokenizerEvidence>,
) -> PromptContractSummary {
    let input_roles = input_roles(node);
    PromptContractSummary {
        protected_input_count: input_roles
            .iter()
            .filter(|role| role.is_protected())
            .count(),
        input_roles,
        tokenizer: tokenizer
            .and_then(TokenizerEvidence::name)
            .map(ToOwned::to_owned),
        shared_prefix_est_tokens: node
            .get_attribute(graph_attrs::SHARED_PREFIX_EST_TOKENS)
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok()),
    }
}

fn input_roles(node: &Node) -> Vec<PromptInputRole> {
    let input_count = node.input_tokens.len();
    node.get_attribute(graph_attrs::INPUT_ROLES)
        .and_then(string_array)
        .and_then(|roles| graph_attrs::parse_prompt_input_roles(input_count, roles).ok())
        .unwrap_or_default()
}

fn string_array(value: &Value) -> Option<Vec<&str>> {
    match value {
        Value::Array(values) => values.iter().map(Value::as_str).collect(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_core::types::{AISOperationType, execution::Node};

    #[test]
    fn summary_does_not_invent_roles_for_missing_or_malformed_contracts() {
        let mut node = Node::new(1, AISOperationType::Ask);
        node.add_input_token(7);

        assert!(summarize(&node, None).input_roles.is_empty());

        node.attributes.insert(
            graph_attrs::INPUT_ROLES.to_string(),
            Value::Array(vec![Value::String("assistant".to_string())]),
        );
        assert!(summarize(&node, None).input_roles.is_empty());
    }

    #[test]
    fn summary_preserves_only_explicit_canonical_roles() {
        let mut node = Node::new(1, AISOperationType::Ask);
        node.add_input_token(7);
        node.add_input_token(8);
        node.attributes.insert(
            graph_attrs::INPUT_ROLES.to_string(),
            Value::Array(vec![
                Value::String("user".to_string()),
                Value::String("system".to_string()),
            ]),
        );

        let summary = summarize(&node, None);
        assert_eq!(
            summary.input_roles,
            vec![PromptInputRole::User, PromptInputRole::System]
        );
        assert_eq!(summary.protected_input_count, 1);
    }
}
