//! Bind Tool Handlers Pass
//!
//! Copies `python_handler_id` from `REGISTER_CAPABILITY` nodes onto their
//! matching `INV_TOOL` nodes so the runtime can dispatch to the Python tool
//! worker without a registry lookup.
//!
//! Runs AFTER `tool-binding-check` (which validates that every INV_TOOL
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
pub const BIND_TOOL_HANDLERS_PASS_NAME: &str = "bind-tool-handlers";

/// Copy `python_handler_id` from REGISTER_CAPABILITY nodes onto matching
/// INV_TOOL nodes.
///
/// Returns the number of INV_TOOL nodes annotated, or an error if conflicting
/// handler IDs are found for the same capability name.
pub fn bind_tool_handlers(module: &mut AirModule) -> Result<usize> {
    // Phase 1: Build capability_name -> python_handler_id map from
    // REGISTER_CAPABILITY nodes. Detect conflicts.
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

    // Phase 2: Stamp python_handler_id onto matching INV_TOOL nodes.
    let mut annotated = 0;
    for node in &mut module.nodes {
        if node.op != AISOperationType::InvTool {
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

/// Same logic as [`bind_tool_handlers`] but on a post-MLIR [`ExecutionDag`].
///
/// Returns the number of INV_TOOL nodes annotated, or `Err` on conflicting
/// handler IDs (E714).
pub fn bind_python_handlers_to_dag(dag: &mut ExecutionDag) -> Result<usize> {
    // Phase 1: Build capability_name -> python_handler_id map.
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

    // Phase 2: Stamp python_handler_id onto matching INV_TOOL nodes.
    let mut annotated = 0;
    for node in &mut dag.nodes {
        if node.op_type != AISOperationType::InvTool {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::air_builder::{AirModule, AirNode};
    use std::collections::HashMap;

    fn empty_module() -> AirModule {
        AirModule {
            name: "test".to_string(),
            nodes: vec![],
            edges: vec![],
            parameters: vec![],
            metadata: HashMap::new(),
        }
    }

    fn reg_cap_node(id: u64, name: &str, cap_name: &str) -> AirNode {
        let mut attrs = HashMap::new();
        attrs.insert(
            attrs::CAPABILITY_NAME.to_string(),
            Value::String(cap_name.to_string()),
        );
        AirNode {
            id,
            name: name.to_string(),
            op: AISOperationType::RegisterCapability,
            attributes: attrs,
        }
    }

    fn reg_cap_with_handler(id: u64, name: &str, cap_name: &str, handler_id: &str) -> AirNode {
        let mut attrs = HashMap::new();
        attrs.insert(
            attrs::CAPABILITY_NAME.to_string(),
            Value::String(cap_name.to_string()),
        );
        attrs.insert(
            attrs::PYTHON_HANDLER_ID.to_string(),
            Value::String(handler_id.to_string()),
        );
        AirNode {
            id,
            name: name.to_string(),
            op: AISOperationType::RegisterCapability,
            attributes: attrs,
        }
    }

    fn inv_tool_node(id: u64, name: &str, capability: &str) -> AirNode {
        let mut attrs = HashMap::new();
        attrs.insert(
            attrs::CAPABILITY.to_string(),
            Value::String(capability.to_string()),
        );
        AirNode {
            id,
            name: name.to_string(),
            op: AISOperationType::InvTool,
            attributes: attrs,
        }
    }

    // ---- Basic copying ----

    #[test]
    fn copies_handler_id_to_inv_tool() {
        let handler = format!("sha256:{}", "a".repeat(64));
        let mut module = empty_module();
        module
            .nodes
            .push(reg_cap_with_handler(1, "reg_add", "add", &handler));
        module.nodes.push(inv_tool_node(2, "call_add", "add"));

        let n = bind_tool_handlers(&mut module).unwrap();
        assert_eq!(n, 1);
        assert_eq!(
            module.nodes[1]
                .attributes
                .get(attrs::PYTHON_HANDLER_ID)
                .and_then(|v| v.as_str()),
            Some(handler.as_str()),
        );
    }

    #[test]
    fn copies_to_multiple_inv_tools_same_capability() {
        let handler = format!("sha256:{}", "b".repeat(64));
        let mut module = empty_module();
        module
            .nodes
            .push(reg_cap_with_handler(1, "reg_calc", "calc", &handler));
        module.nodes.push(inv_tool_node(2, "call_calc_1", "calc"));
        module.nodes.push(inv_tool_node(3, "call_calc_2", "calc"));

        let n = bind_tool_handlers(&mut module).unwrap();
        assert_eq!(n, 2);
        for i in 1..=2 {
            assert_eq!(
                module.nodes[i]
                    .attributes
                    .get(attrs::PYTHON_HANDLER_ID)
                    .and_then(|v| v.as_str()),
                Some(handler.as_str()),
            );
        }
    }

    #[test]
    fn copies_different_handlers_for_different_capabilities() {
        let handler_a = format!("sha256:{}", "a".repeat(64));
        let handler_b = format!("sha256:{}", "b".repeat(64));
        let mut module = empty_module();
        module
            .nodes
            .push(reg_cap_with_handler(1, "reg_a", "tool_a", &handler_a));
        module
            .nodes
            .push(reg_cap_with_handler(2, "reg_b", "tool_b", &handler_b));
        module.nodes.push(inv_tool_node(3, "call_a", "tool_a"));
        module.nodes.push(inv_tool_node(4, "call_b", "tool_b"));

        let n = bind_tool_handlers(&mut module).unwrap();
        assert_eq!(n, 2);
        assert_eq!(
            module.nodes[2]
                .attributes
                .get(attrs::PYTHON_HANDLER_ID)
                .and_then(|v| v.as_str()),
            Some(handler_a.as_str()),
        );
        assert_eq!(
            module.nodes[3]
                .attributes
                .get(attrs::PYTHON_HANDLER_ID)
                .and_then(|v| v.as_str()),
            Some(handler_b.as_str()),
        );
    }

    // ---- No-op cases ----

    #[test]
    fn no_op_when_no_python_handler_id() {
        let mut module = empty_module();
        module.nodes.push(reg_cap_node(1, "reg_tool", "my_tool"));
        module.nodes.push(inv_tool_node(2, "call_tool", "my_tool"));

        let n = bind_tool_handlers(&mut module).unwrap();
        assert_eq!(n, 0);
        assert!(
            module.nodes[1]
                .attributes
                .get(attrs::PYTHON_HANDLER_ID)
                .is_none(),
        );
    }

    #[test]
    fn no_op_on_empty_module() {
        let mut module = empty_module();
        let n = bind_tool_handlers(&mut module).unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn no_op_on_builtin_capability() {
        let mut module = empty_module();
        module.nodes.push(inv_tool_node(1, "call_bash", "bash"));

        let n = bind_tool_handlers(&mut module).unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn skips_non_tool_nodes() {
        let mut module = empty_module();
        module.nodes.push(AirNode {
            id: 1,
            name: "ask_node".to_string(),
            op: AISOperationType::Ask,
            attributes: HashMap::new(),
        });
        let n = bind_tool_handlers(&mut module).unwrap();
        assert_eq!(n, 0);
    }

    // ---- Duplicate registrations (same handler = OK) ----

    #[test]
    fn duplicate_registration_same_handler_is_ok() {
        let handler = format!("sha256:{}", "c".repeat(64));
        let mut module = empty_module();
        module
            .nodes
            .push(reg_cap_with_handler(1, "reg_1", "tool", &handler));
        module
            .nodes
            .push(reg_cap_with_handler(2, "reg_2", "tool", &handler));
        module.nodes.push(inv_tool_node(3, "call_tool", "tool"));

        let n = bind_tool_handlers(&mut module).unwrap();
        assert_eq!(n, 1);
        assert_eq!(
            module.nodes[2]
                .attributes
                .get(attrs::PYTHON_HANDLER_ID)
                .and_then(|v| v.as_str()),
            Some(handler.as_str()),
        );
    }

    // ---- E714: conflicting handler IDs ----

    #[test]
    fn e714_conflicting_handler_ids_errors() {
        let handler_a = format!("sha256:{}", "a".repeat(64));
        let handler_b = format!("sha256:{}", "b".repeat(64));
        let mut module = empty_module();
        module
            .nodes
            .push(reg_cap_with_handler(1, "reg_1", "tool", &handler_a));
        module
            .nodes
            .push(reg_cap_with_handler(2, "reg_2", "tool", &handler_b));
        module.nodes.push(inv_tool_node(3, "call_tool", "tool"));

        let result = bind_tool_handlers(&mut module);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Conflicting"));
        assert!(err.contains("tool"));
    }

    // ---- ExecutionDag variant ----

    use apxm_core::types::Node;
    use apxm_core::types::execution::ExecutionDag;

    fn dag_reg_cap_with_handler(id: u64, cap_name: &str, handler_id: &str) -> Node {
        let mut n = Node::new(id, AISOperationType::RegisterCapability);
        n.attributes.insert(
            attrs::CAPABILITY_NAME.to_string(),
            Value::String(cap_name.to_string()),
        );
        n.attributes.insert(
            attrs::PYTHON_HANDLER_ID.to_string(),
            Value::String(handler_id.to_string()),
        );
        n
    }

    fn dag_inv_tool(id: u64, capability: &str) -> Node {
        let mut n = Node::new(id, AISOperationType::InvTool);
        n.attributes.insert(
            attrs::CAPABILITY.to_string(),
            Value::String(capability.to_string()),
        );
        n
    }

    #[test]
    fn dag_copies_handler_id_to_inv_tool() {
        let handler = format!("sha256:{}", "a".repeat(64));
        let mut dag = ExecutionDag::new();
        dag.nodes.push(dag_reg_cap_with_handler(1, "add", &handler));
        dag.nodes.push(dag_inv_tool(2, "add"));

        let n = bind_python_handlers_to_dag(&mut dag).unwrap();
        assert_eq!(n, 1);
        assert_eq!(
            dag.nodes[1]
                .attributes
                .get(attrs::PYTHON_HANDLER_ID)
                .and_then(|v| v.as_str()),
            Some(handler.as_str()),
        );
    }

    #[test]
    fn dag_no_op_without_handler_id() {
        let mut dag = ExecutionDag::new();
        let mut reg = Node::new(1, AISOperationType::RegisterCapability);
        reg.attributes.insert(
            attrs::CAPABILITY_NAME.to_string(),
            Value::String("my_tool".to_string()),
        );
        dag.nodes.push(reg);
        dag.nodes.push(dag_inv_tool(2, "my_tool"));

        let n = bind_python_handlers_to_dag(&mut dag).unwrap();
        assert_eq!(n, 0);
        assert!(
            dag.nodes[1]
                .attributes
                .get(attrs::PYTHON_HANDLER_ID)
                .is_none(),
        );
    }

    #[test]
    fn dag_e714_conflicting_handler_ids_errors() {
        let handler_a = format!("sha256:{}", "a".repeat(64));
        let handler_b = format!("sha256:{}", "b".repeat(64));
        let mut dag = ExecutionDag::new();
        dag.nodes
            .push(dag_reg_cap_with_handler(1, "tool", &handler_a));
        dag.nodes
            .push(dag_reg_cap_with_handler(2, "tool", &handler_b));
        dag.nodes.push(dag_inv_tool(3, "tool"));

        let result = bind_python_handlers_to_dag(&mut dag);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Conflicting"));
        assert!(err.contains("tool"));
    }
}
