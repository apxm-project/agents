//! NEGOTIATE operation - Multi-agent negotiation protocol
//!
//! Initiates a negotiation among multiple parties. Each party is sent the
//! proposal via their flow, and responses are collected. Basic implementation
//! that runs a single round and returns all responses.
//!
//! ## Attributes
//! - `parties`    (required): JSON array of agent names
//! - `proposal`   (required): the proposal to negotiate on
//! - `max_rounds` (optional): maximum negotiation rounds (default: 3)

use super::{ExecutionContext, Node, Result, Value, get_string_attribute};
use crate::aam::TransitionLabel;
use crate::executor::ExecutorEngine;
use apxm_core::constants::defaults;
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::constants::runtime::{belief_keys, metadata, response_keys, transition_labels};
use apxm_core::error::RuntimeError;
use std::collections::HashMap;

/// Well-known flow names tried when looking up a negotiation party.
const NEGOTIATE_FLOW_NAMES: &[&str] = &["negotiate", "communicate", "main"];

pub async fn execute(ctx: &ExecutionContext, node: &Node, _inputs: Vec<Value>) -> Result<Value> {
    let proposal = get_string_attribute(node, graph_attrs::PROPOSAL)?;

    // Parse parties from attribute
    let parties_value = node.attributes.get(graph_attrs::PARTIES).cloned().ok_or_else(|| {
        RuntimeError::Operation {
            op_type: node.op_type,
            message: "Missing required attribute: parties".to_string(),
        }
    })?;

    let parties: Vec<String> = match &parties_value {
        Value::Array(arr) => arr
            .iter()
            .filter_map(|v| v.as_string().map(|s| s.to_string()))
            .collect(),
        Value::String(s) => {
            // Try parsing as JSON array
            serde_json::from_str::<Vec<String>>(s).unwrap_or_else(|_| vec![s.clone()])
        }
        _ => {
            return Err(RuntimeError::Operation {
                op_type: node.op_type,
                message: "Attribute 'parties' must be an array of agent names".to_string(),
            });
        }
    };

    let max_rounds = node
        .attributes
        .get(graph_attrs::MAX_ROUNDS)
        .and_then(|v| v.as_u64())
        .unwrap_or(defaults::DEFAULT_MAX_NEGOTIATE_ROUNDS as u64) as usize;

    tracing::info!(
        execution_id = %ctx.execution_id,
        parties = ?parties,
        proposal = %proposal,
        max_rounds = %max_rounds,
        "Executing NEGOTIATE operation"
    );

    // Record negotiation in AAM
    ctx.aam.set_belief(
        belief_keys::NEGOTIATE_ACTIVE.to_string(),
        Value::String(proposal.clone()),
        TransitionLabel::Custom(transition_labels::NEGOTIATE_START.to_string()),
    );

    // Basic negotiation: send proposal to each party and collect responses
    let mut responses = HashMap::new();
    let mut consensus_reached = false;

    for round in 0..max_rounds {
        tracing::debug!(round = round, "NEGOTIATE round");

        for party in &parties {
            // Look up party's flow
            let sub_dag = {
                let mut found = None;
                for flow_name in NEGOTIATE_FLOW_NAMES {
                    if let Some(dag) = ctx.flow_registry.get_flow(party, flow_name) {
                        found = Some(dag);
                        break;
                    }
                }
                match found {
                    Some(dag) => dag,
                    None => {
                        tracing::warn!(party = %party, "No flow found for negotiation party");
                        responses.insert(
                            party.clone(),
                            Value::String(format!("Agent '{}' not available", party)),
                        );
                        continue;
                    }
                }
            };

            let child_ctx = ctx
                .child()
                .with_metadata(metadata::NEGOTIATE_PROPOSAL.to_string(), proposal.clone())
                .with_metadata(metadata::NEGOTIATE_ROUND.to_string(), round.to_string())
                .with_metadata(metadata::NEGOTIATE_PARTY.to_string(), party.clone());

            let _ = child_ctx
                .memory
                .write(
                    crate::memory::MemorySpace::Stm,
                    belief_keys::NEGOTIATE_PROPOSAL.to_string(),
                    Value::String(proposal.clone()),
                )
                .await;

            let engine = ExecutorEngine::new(child_ctx);
            let dag_to_execute = (*sub_dag).clone();

            match engine.execute_dag(dag_to_execute).await {
                Ok(result) => {
                    let response = result
                        .results
                        .values()
                        .find(|v| !matches!(v, Value::Null))
                        .cloned()
                        .unwrap_or(Value::Null);
                    responses.insert(party.clone(), response);
                }
                Err(e) => {
                    tracing::warn!(party = %party, error = %e, "NEGOTIATE party failed");
                    responses.insert(
                        party.clone(),
                        Value::String(format!("Error: {}", e)),
                    );
                }
            }
        }

        // Basic consensus check: if all parties responded with non-error values
        consensus_reached = responses.values().all(|v| {
            !matches!(v, Value::Null) && !v.as_string().is_some_and(|s| s.starts_with("Error:"))
        });

        if consensus_reached {
            break;
        }
    }

    // Clear negotiation state
    ctx.aam.set_belief(
        belief_keys::NEGOTIATE_ACTIVE.to_string(),
        Value::Null,
        TransitionLabel::Custom(transition_labels::NEGOTIATE_COMPLETE.to_string()),
    );

    // Build result
    let mut result = HashMap::new();
    result.insert(
        response_keys::CONSENSUS.to_string(),
        Value::Bool(consensus_reached),
    );
    result.insert(response_keys::PROPOSAL.to_string(), Value::String(proposal));
    result.insert(response_keys::RESPONSES.to_string(), Value::Object(responses));

    tracing::info!(
        execution_id = %ctx.execution_id,
        consensus = consensus_reached,
        "NEGOTIATE completed"
    );

    Ok(Value::Object(result))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::CapabilitySystem;
    use crate::capability::flow_registry::FlowRegistry;
    use crate::memory::{MemoryConfig, MemorySystem};
    use apxm_backends::LLMRegistry;
    use apxm_core::constants::graph::attrs as graph_attrs;
    use apxm_core::constants::runtime::response_keys;
    use apxm_core::types::{
        execution::{ExecutionDag, NodeMetadata},
        operations::AISOperationType,
    };
    use std::collections::HashMap;
    use std::sync::Arc;

    /// Create a simple flow DAG that returns a constant response.
    fn create_negotiate_flow_dag(response_text: &str) -> ExecutionDag {
        let mut const_node = apxm_core::types::execution::Node {
            id: 1,
            op_type: AISOperationType::ConstStr,
            attributes: HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![100],
            metadata: NodeMetadata::default(),
        };
        const_node.attributes.insert(
            graph_attrs::VALUE.to_string(),
            Value::String(response_text.to_string()),
        );

        ExecutionDag {
            nodes: vec![const_node],
            edges: vec![],
            entry_nodes: vec![1],
            exit_nodes: vec![1],
            metadata: Default::default(),
        }
    }

    fn make_negotiate_node(parties: Vec<&str>, proposal: &str) -> apxm_core::types::execution::Node {
        let mut node = apxm_core::types::execution::Node {
            id: 1,
            op_type: AISOperationType::Negotiate,
            attributes: HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![100],
            metadata: NodeMetadata::default(),
        };
        node.attributes.insert(
            graph_attrs::PROPOSAL.to_string(),
            Value::String(proposal.to_string()),
        );
        node.attributes.insert(
            graph_attrs::PARTIES.to_string(),
            Value::Array(
                parties
                    .iter()
                    .map(|p| Value::String(p.to_string()))
                    .collect(),
            ),
        );
        node
    }

    #[tokio::test]
    async fn test_negotiate_basic_round_with_consensus() {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());
        let flow_registry = Arc::new(FlowRegistry::new());

        // Register negotiate flows for two parties
        flow_registry.register_flow("alice", "negotiate", create_negotiate_flow_dag("I agree"));
        flow_registry.register_flow("bob", "negotiate", create_negotiate_flow_dag("Accepted"));

        let ctx = ExecutionContext::new(
            memory,
            llm_registry,
            capability_system,
            crate::aam::Aam::new(),
        );
        let ctx = ExecutionContext {
            flow_registry,
            ..ctx
        };

        let node = make_negotiate_node(vec!["alice", "bob"], "Let's collaborate");
        let result = execute(&ctx, &node, vec![]).await.unwrap();

        match &result {
            Value::Object(obj) => {
                // Should have consensus
                assert_eq!(
                    obj.get(response_keys::CONSENSUS),
                    Some(&Value::Bool(true)),
                    "Both parties responded successfully => consensus"
                );
                // Should contain the proposal
                assert_eq!(
                    obj.get(response_keys::PROPOSAL),
                    Some(&Value::String("Let's collaborate".to_string()))
                );
                // Should have responses from both parties
                let responses = obj.get(response_keys::RESPONSES).unwrap();
                match responses {
                    Value::Object(resp_obj) => {
                        assert!(resp_obj.contains_key("alice"));
                        assert!(resp_obj.contains_key("bob"));
                    }
                    _ => panic!("Expected responses to be an Object"),
                }
            }
            _ => panic!("Expected Value::Object, got {:?}", result),
        }
    }

    #[tokio::test]
    async fn test_negotiate_party_not_found() {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());
        let flow_registry = Arc::new(FlowRegistry::new());

        // Register only alice, not bob
        flow_registry.register_flow("alice", "negotiate", create_negotiate_flow_dag("OK"));

        let ctx = ExecutionContext::new(
            memory,
            llm_registry,
            capability_system,
            crate::aam::Aam::new(),
        );
        let ctx = ExecutionContext {
            flow_registry,
            ..ctx
        };

        let node = make_negotiate_node(vec!["alice", "bob"], "some proposal");
        let result = execute(&ctx, &node, vec![]).await.unwrap();

        match &result {
            Value::Object(obj) => {
                // Consensus should be false because bob's response starts with "Agent"
                // (it's a not-available string, not an error, so consensus check looks at it)
                let responses = obj.get(response_keys::RESPONSES).unwrap();
                match responses {
                    Value::Object(resp_obj) => {
                        let bob_response = resp_obj.get("bob").unwrap();
                        assert!(
                            bob_response.as_string().unwrap().contains("not available"),
                            "Bob should be reported as not available"
                        );
                    }
                    _ => panic!("Expected responses to be Object"),
                }
            }
            _ => panic!("Expected Value::Object"),
        }
    }

    #[tokio::test]
    async fn test_negotiate_missing_proposal() {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());

        let ctx = ExecutionContext::new(
            memory,
            llm_registry,
            capability_system,
            crate::aam::Aam::new(),
        );

        // Node without proposal attribute
        let mut node = apxm_core::types::execution::Node {
            id: 1,
            op_type: AISOperationType::Negotiate,
            attributes: HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![100],
            metadata: NodeMetadata::default(),
        };
        node.attributes.insert(
            graph_attrs::PARTIES.to_string(),
            Value::Array(vec![Value::String("alice".to_string())]),
        );

        let result = execute(&ctx, &node, vec![]).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("proposal"));
    }

    #[tokio::test]
    async fn test_negotiate_missing_parties() {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());

        let ctx = ExecutionContext::new(
            memory,
            llm_registry,
            capability_system,
            crate::aam::Aam::new(),
        );

        let mut node = apxm_core::types::execution::Node {
            id: 1,
            op_type: AISOperationType::Negotiate,
            attributes: HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![100],
            metadata: NodeMetadata::default(),
        };
        node.attributes.insert(
            graph_attrs::PROPOSAL.to_string(),
            Value::String("a proposal".to_string()),
        );

        let result = execute(&ctx, &node, vec![]).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("parties"));
    }

    #[tokio::test]
    async fn test_negotiate_clears_aam_belief_after_completion() {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());
        let flow_registry = Arc::new(FlowRegistry::new());

        flow_registry.register_flow("alice", "negotiate", create_negotiate_flow_dag("yes"));

        let aam = crate::aam::Aam::new();
        let ctx = ExecutionContext::new(memory, llm_registry, capability_system, aam.clone());
        let ctx = ExecutionContext {
            flow_registry,
            ..ctx
        };

        let node = make_negotiate_node(vec!["alice"], "test proposal");
        let _ = execute(&ctx, &node, vec![]).await.unwrap();

        // Negotiate active belief should be cleared
        let beliefs = ctx.aam.beliefs();
        let negotiate_belief = beliefs.get(belief_keys::NEGOTIATE_ACTIVE);
        assert!(
            negotiate_belief.is_none() || matches!(negotiate_belief, Some(Value::Null)),
            "Negotiate active belief should be cleared after completion"
        );
    }

    #[tokio::test]
    async fn test_negotiate_uses_communicate_flow_fallback() {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());
        let flow_registry = Arc::new(FlowRegistry::new());

        // Register as "communicate" flow (second priority in NEGOTIATE_FLOW_NAMES)
        flow_registry.register_flow("alice", "communicate", create_negotiate_flow_dag("agreed"));

        let ctx = ExecutionContext::new(
            memory,
            llm_registry,
            capability_system,
            crate::aam::Aam::new(),
        );
        let ctx = ExecutionContext {
            flow_registry,
            ..ctx
        };

        let node = make_negotiate_node(vec!["alice"], "test");
        let result = execute(&ctx, &node, vec![]).await;
        assert!(result.is_ok(), "Should succeed with 'communicate' flow fallback");
    }
}
