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
use apxm_core::error::RuntimeError;
use std::collections::HashMap;

pub async fn execute(ctx: &ExecutionContext, node: &Node, _inputs: Vec<Value>) -> Result<Value> {
    let proposal = get_string_attribute(node, "proposal")?;

    // Parse parties from attribute
    let parties_value = node.attributes.get("parties").cloned().ok_or_else(|| {
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
        .get("max_rounds")
        .and_then(|v| v.as_u64())
        .unwrap_or(3) as usize;

    tracing::info!(
        execution_id = %ctx.execution_id,
        parties = ?parties,
        proposal = %proposal,
        max_rounds = %max_rounds,
        "Executing NEGOTIATE operation"
    );

    // Record negotiation in AAM
    ctx.aam.set_belief(
        "_negotiate_active".to_string(),
        Value::String(proposal.clone()),
        TransitionLabel::Custom("negotiate_start".to_string()),
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
                for flow_name in &["negotiate", "communicate", "main"] {
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
                .with_metadata("negotiate_proposal".to_string(), proposal.clone())
                .with_metadata("negotiate_round".to_string(), round.to_string())
                .with_metadata("negotiate_party".to_string(), party.clone());

            let _ = child_ctx
                .memory
                .write(
                    crate::memory::MemorySpace::Stm,
                    "_negotiate_proposal".to_string(),
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
            !matches!(v, Value::Null) && !v.as_string().map_or(false, |s| s.starts_with("Error:"))
        });

        if consensus_reached {
            break;
        }
    }

    // Clear negotiation state
    ctx.aam.set_belief(
        "_negotiate_active".to_string(),
        Value::Null,
        TransitionLabel::Custom("negotiate_complete".to_string()),
    );

    // Build result
    let mut result = HashMap::new();
    result.insert(
        "consensus".to_string(),
        Value::Bool(consensus_reached),
    );
    result.insert("proposal".to_string(), Value::String(proposal));
    result.insert("responses".to_string(), Value::Object(responses));

    tracing::info!(
        execution_id = %ctx.execution_id,
        consensus = consensus_reached,
        "NEGOTIATE completed"
    );

    Ok(Value::Object(result))
}
