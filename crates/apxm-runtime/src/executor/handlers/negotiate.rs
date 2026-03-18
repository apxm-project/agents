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
