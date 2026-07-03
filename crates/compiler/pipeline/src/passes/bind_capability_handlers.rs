//! Bind Tool Handlers Pass
//!
//! Copies `python_handler_id` from `REGISTER_CAPABILITY` nodes onto their
//! matching `INV_CAP` nodes so the runtime can dispatch to the Python tool
//! worker without a registry lookup.
//!
//! Runs AFTER `capability-binding-check` (which validates that every INV_CAP
//! resolves) and BEFORE lowering.
//!
//! Diagnostics:
//! - E714: multiple REGISTER_CAPABILITYs for the same capability name carry
//!   conflicting `python_handler_id` values

use crate::air_builder::AirModule;
use apxm_core::constants::graph::attrs;
use apxm_core::error::compiler::{CompilerError, Result};
use apxm_core::error::span::Span;
use apxm_core::error::{Error, ErrorCode};
use apxm_core::types::execution::ExecutionDag;
use apxm_core::types::{AISOperationType, Value};
use std::collections::HashMap;

/// Pass name sentinel for pipeline ordering and diagnostics.
pub const BIND_CAPABILITY_HANDLERS_PASS_NAME: &str = "bind-capability-handlers";

/// Copy `python_handler_id` from REGISTER_CAPABILITY nodes onto matching
/// INV_CAP nodes.
///
/// Returns the number of INV_CAP nodes annotated, or an error if conflicting
/// handler IDs are found for the same capability name.
pub fn bind_capability_handlers(module: &mut AirModule) -> Result<usize> {
    // Pass 1: build capability_name -> python_handler_id map from REGISTER_CAPABILITY nodes. Detect conflicts.
    let mut handler_map: HashMap<String, String> = HashMap::new();
    let mut conflicts: Vec<String> = Vec::new();

    for node in &module.nodes {
        if node.op != AISOperationType::RegisterCapability {
            continue;
        }
        let Some(cap_name) = node
            .attributes
            .get(attrs::CAPABILITY_NAME)
            .and_then(|v| v.as_str())
        else {
            continue;
        };
        let Some(handler_id) = node
            .attributes
            .get(attrs::PYTHON_HANDLER_ID)
            .and_then(|v| v.as_str())
        else {
            continue;
        };

        match handler_map.get(cap_name) {
            Some(existing) if existing != handler_id => {
                conflicts.push(format!(
                    "capability '{}': REGISTER_CAPABILITY node '{}' has python_handler_id \
                     '{}' but a previous registration uses '{}'",
                    cap_name, node.name, handler_id, existing,
                ));
            }
            None => {
                handler_map.insert(cap_name.to_string(), handler_id.to_string());
            }
            _ => {
                // Same handler_id, no conflict.
            }
        }
    }

    if !conflicts.is_empty() {
        let msg = format!(
            "Conflicting python_handler_id values:\n\n{}",
            conflicts.join("\n"),
        );
        return Err(CompilerError::Verification(Box::new(Error::new(
            ErrorCode::ConflictingHandlerId,
            msg,
            Span::new("<graph>".to_string(), 1, 1, 0),
        ))));
    }

    // Pass 2: stamp python_handler_id onto matching INV_CAP nodes.
    let mut annotated = 0;
    for node in &mut module.nodes {
        if node.op != AISOperationType::InvCap {
            continue;
        }
        let Some(cap_name) = node
            .attributes
            .get(attrs::CAPABILITY)
            .and_then(|v| v.as_str())
            .map(str::to_owned)
        else {
            continue;
        };

        if let Some(handler_id) = handler_map.get(&cap_name) {
            node.attributes.insert(
                attrs::PYTHON_HANDLER_ID.to_string(),
                Value::String(handler_id.clone()),
            );
            annotated += 1;
        }
    }

    Ok(annotated)
}

/// Same logic as [`bind_capability_handlers`] but on a post-MLIR [`ExecutionDag`].
///
/// Returns the number of INV_CAP nodes annotated, or `Err` on conflicting
/// handler IDs (E714).
pub fn bind_python_handlers_to_dag(dag: &mut ExecutionDag) -> Result<usize> {
    // Pass 1: build capability_name -> python_handler_id map.
    let mut handler_map: HashMap<String, String> = HashMap::new();
    let mut conflicts: Vec<String> = Vec::new();

    for node in &dag.nodes {
        if node.op_type != AISOperationType::RegisterCapability {
            continue;
        }
        let Some(cap_name) = node
            .attributes
            .get(attrs::CAPABILITY_NAME)
            .and_then(|v| v.as_str())
        else {
            continue;
        };
        let Some(handler_id) = node
            .attributes
            .get(attrs::PYTHON_HANDLER_ID)
            .and_then(|v| v.as_str())
        else {
            continue;
        };

        match handler_map.get(cap_name) {
            Some(existing) if existing != handler_id => {
                conflicts.push(format!(
                    "capability '{}': REGISTER_CAPABILITY node #{} has python_handler_id \
                     '{}' but a previous registration uses '{}'",
                    cap_name, node.id, handler_id, existing,
                ));
            }
            None => {
                handler_map.insert(cap_name.to_string(), handler_id.to_string());
            }
            _ => {}
        }
    }

    if !conflicts.is_empty() {
        let msg = format!(
            "Conflicting python_handler_id values:\n\n{}",
            conflicts.join("\n"),
        );
        return Err(CompilerError::Verification(Box::new(Error::new(
            ErrorCode::ConflictingHandlerId,
            msg,
            Span::new("<artifact>".to_string(), 1, 1, 0),
        ))));
    }

    // Pass 2: stamp python_handler_id onto matching INV_CAP nodes.
    let mut annotated = 0;
    for node in &mut dag.nodes {
        if node.op_type != AISOperationType::InvCap {
            continue;
        }
        let Some(cap_name) = node
            .attributes
            .get(attrs::CAPABILITY)
            .and_then(|v| v.as_str())
            .map(str::to_owned)
        else {
            continue;
        };

        if let Some(handler_id) = handler_map.get(&cap_name) {
            node.attributes.insert(
                attrs::PYTHON_HANDLER_ID.to_string(),
                Value::String(handler_id.clone()),
            );
            annotated += 1;
        }
    }

    Ok(annotated)
}
