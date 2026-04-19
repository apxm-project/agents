//! Tool Binding Check Pass
//!
//! Validates that every `INV_TOOL.capability` resolves to either an upstream
//! `REGISTER_CAPABILITY` in the same module or a known Rust builtin capability.
//! Also validates `python_handler_id` format and warns on unused capabilities.
//!
//! Diagnostics:
//! - E712: unbound capability (INV_TOOL references unknown tool)
//! - E713: invalid `python_handler_id` format (must be `sha256:<hex64>`)
//! - W721: unused capability (REGISTER_CAPABILITY never invoked)

use crate::air_builder::AirModule;
use apxm_ais::attrs;
use apxm_core::error::compiler::{CompilerError, Result};
use apxm_core::error::span::Span;
use apxm_core::error::{Error, ErrorCode};
use apxm_core::types::AISOperationType;
use std::collections::{HashMap, HashSet};

/// Pass name sentinel for pipeline ordering and diagnostics.
pub const TOOL_BINDING_PASS_NAME: &str = "tool-binding-check";

/// Known builtin capabilities provided by the Rust runtime.
const BUILTIN_CAPABILITIES: &[&str] = &["bash", "read", "write", "search_web"];

/// Regex-equivalent validation for `sha256:<64 hex chars>`.
fn is_valid_handler_id(s: &str) -> bool {
    if let Some(hex) = s.strip_prefix("sha256:") {
        hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit())
    } else {
        false
    }
}

/// Diagnostic emitted by the tool binding check pass.
#[derive(Debug, Clone)]
pub struct ToolBindingDiagnostic {
    pub code: ErrorCode,
    pub message: String,
    pub node_name: String,
}

/// Run the tool binding check pass on a module.
///
/// Returns `Ok(diagnostics)` where diagnostics may contain warnings,
/// or `Err(CompilerError)` if hard errors (E712, E713) are found.
pub fn tool_binding_check(module: &AirModule) -> Result<Vec<ToolBindingDiagnostic>> {
    let mut errors: Vec<ToolBindingDiagnostic> = Vec::new();
    let mut warnings: Vec<ToolBindingDiagnostic> = Vec::new();

    // Collect all registered capability names from REGISTER_CAPABILITY nodes.
    let mut registered: HashMap<String, Vec<String>> = HashMap::new();
    for node in &module.nodes {
        if node.op == AISOperationType::RegisterCapability {
            if let Some(name) = node.attributes.get(attrs::CAPABILITY_NAME).and_then(|v| v.as_str())
            {
                registered
                    .entry(name.to_string())
                    .or_default()
                    .push(node.name.clone());
            }
        }
    }

    // Collect all invoked capability names from INV_TOOL nodes.
    let mut invoked: HashSet<String> = HashSet::new();
    for node in &module.nodes {
        if node.op == AISOperationType::InvTool {
            if let Some(cap) = node.attributes.get(attrs::CAPABILITY).and_then(|v| v.as_str()) {
                invoked.insert(cap.to_string());

                // E712: capability must resolve to a REGISTER_CAPABILITY or a builtin.
                if !registered.contains_key(cap) && !BUILTIN_CAPABILITIES.contains(&cap) {
                    errors.push(ToolBindingDiagnostic {
                        code: ErrorCode::UnboundCapability,
                        message: format!(
                            "INV_TOOL node '{}' references capability '{}' which is not \
                             registered by any REGISTER_CAPABILITY node and is not a \
                             known builtin ({})",
                            node.name,
                            cap,
                            BUILTIN_CAPABILITIES.join(", "),
                        ),
                        node_name: node.name.clone(),
                    });
                }
            }
        }
    }

    // E713: validate python_handler_id format on REGISTER_CAPABILITY nodes.
    for node in &module.nodes {
        if node.op == AISOperationType::RegisterCapability {
            if let Some(handler_id) = node
                .attributes
                .get(attrs::PYTHON_HANDLER_ID)
                .and_then(|v| v.as_str())
            {
                if !is_valid_handler_id(handler_id) {
                    errors.push(ToolBindingDiagnostic {
                        code: ErrorCode::InvalidHandlerId,
                        message: format!(
                            "REGISTER_CAPABILITY node '{}' has invalid python_handler_id \
                             '{}'; expected format: sha256:<64 hex chars>",
                            node.name, handler_id,
                        ),
                        node_name: node.name.clone(),
                    });
                }
            }
        }
    }

    // W721: warn on REGISTER_CAPABILITY whose name is never invoked.
    for (cap_name, reg_nodes) in &registered {
        if !invoked.contains(cap_name) {
            for reg_node in reg_nodes {
                warnings.push(ToolBindingDiagnostic {
                    code: ErrorCode::UnusedCapability,
                    message: format!(
                        "REGISTER_CAPABILITY '{}' in node '{}' is never invoked by any \
                         INV_TOOL node",
                        cap_name, reg_node,
                    ),
                    node_name: reg_node.clone(),
                });
            }
        }
    }

    // TODO(W214/SchemaDrift): schema-vs-signature drift check when Python
    // frontend is in-process. Requires runtime access to loaded handler
    // signatures which is not available at compile time in MVP.

    if !errors.is_empty() {
        let error_messages: Vec<String> = errors.iter().map(|e| e.message.clone()).collect();
        let combined = format!(
            "Tool binding check failed:\n\n{}",
            error_messages.join("\n"),
        );

        return Err(CompilerError::Verification(Box::new(Error::new(
            errors[0].code,
            combined,
            Span::new("<graph>".to_string(), 1, 1, 0),
        ))));
    }

    Ok(warnings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::air_builder::{AirModule, AirNode};
    use apxm_core::types::Value;
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

    // ---- E712: unbound capability ----

    #[test]
    fn e712_inv_tool_references_registered_capability() {
        let mut module = empty_module();
        module.nodes.push(reg_cap_node(1, "reg_search", "search"));
        module.nodes.push(inv_tool_node(2, "call_search", "search"));
        let result = tool_binding_check(&module);
        assert!(result.is_ok());
    }

    #[test]
    fn e712_inv_tool_references_builtin_capability() {
        let mut module = empty_module();
        for (i, builtin) in BUILTIN_CAPABILITIES.iter().enumerate() {
            module
                .nodes
                .push(inv_tool_node((i + 1) as u64, &format!("call_{builtin}"), builtin));
        }
        let result = tool_binding_check(&module);
        assert!(result.is_ok());
    }

    #[test]
    fn e712_inv_tool_references_unknown_capability() {
        let mut module = empty_module();
        module
            .nodes
            .push(inv_tool_node(1, "call_missing", "nonexistent_tool"));
        let result = tool_binding_check(&module);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("nonexistent_tool"));
        assert!(err.contains("call_missing"));
    }

    #[test]
    fn e712_multiple_unbound_capabilities_all_reported() {
        let mut module = empty_module();
        module
            .nodes
            .push(inv_tool_node(1, "call_a", "missing_a"));
        module
            .nodes
            .push(inv_tool_node(2, "call_b", "missing_b"));
        let result = tool_binding_check(&module);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("missing_a"));
        assert!(err.contains("missing_b"));
    }

    // ---- E713: invalid python_handler_id ----

    #[test]
    fn e713_valid_handler_id_passes() {
        let valid_hash = format!("sha256:{}", "a".repeat(64));
        let mut module = empty_module();
        module.nodes.push(reg_cap_with_handler(
            1,
            "reg_add",
            "add",
            &valid_hash,
        ));
        module.nodes.push(inv_tool_node(2, "call_add", "add"));
        let result = tool_binding_check(&module);
        assert!(result.is_ok());
    }

    #[test]
    fn e713_invalid_handler_id_too_short() {
        let mut module = empty_module();
        module.nodes.push(reg_cap_with_handler(
            1,
            "reg_add",
            "add",
            "sha256:abcd",
        ));
        module.nodes.push(inv_tool_node(2, "call_add", "add"));
        let result = tool_binding_check(&module);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("sha256:abcd"));
        assert!(err.contains("reg_add"));
    }

    #[test]
    fn e713_invalid_handler_id_wrong_prefix() {
        let mut module = empty_module();
        module.nodes.push(reg_cap_with_handler(
            1,
            "reg_add",
            "add",
            &format!("md5:{}", "a".repeat(64)),
        ));
        module.nodes.push(inv_tool_node(2, "call_add", "add"));
        let result = tool_binding_check(&module);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("md5:"));
    }

    #[test]
    fn e713_invalid_handler_id_non_hex() {
        let mut module = empty_module();
        let bad_hex = format!("sha256:{}z", "a".repeat(63));
        module.nodes.push(reg_cap_with_handler(
            1,
            "reg_add",
            "add",
            &bad_hex,
        ));
        module.nodes.push(inv_tool_node(2, "call_add", "add"));
        let result = tool_binding_check(&module);
        assert!(result.is_err());
    }

    #[test]
    fn e713_no_handler_id_is_fine() {
        // REGISTER_CAPABILITY without python_handler_id is valid (Rust builtin capability)
        let mut module = empty_module();
        module.nodes.push(reg_cap_node(1, "reg_tool", "my_tool"));
        module.nodes.push(inv_tool_node(2, "call_tool", "my_tool"));
        let result = tool_binding_check(&module);
        assert!(result.is_ok());
    }

    // ---- W721: unused capability ----

    #[test]
    fn w721_unused_capability_emits_warning() {
        let mut module = empty_module();
        module.nodes.push(reg_cap_node(1, "reg_unused", "unused_tool"));
        let result = tool_binding_check(&module);
        assert!(result.is_ok());
        let warnings = result.unwrap();
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].code, ErrorCode::UnusedCapability);
        assert!(warnings[0].message.contains("unused_tool"));
    }

    #[test]
    fn w721_used_capability_no_warning() {
        let mut module = empty_module();
        module.nodes.push(reg_cap_node(1, "reg_tool", "my_tool"));
        module.nodes.push(inv_tool_node(2, "call_tool", "my_tool"));
        let result = tool_binding_check(&module);
        assert!(result.is_ok());
        let warnings = result.unwrap();
        assert!(warnings.is_empty());
    }

    #[test]
    fn w721_multiple_unused_capabilities() {
        let mut module = empty_module();
        module.nodes.push(reg_cap_node(1, "reg_a", "tool_a"));
        module.nodes.push(reg_cap_node(2, "reg_b", "tool_b"));
        module.nodes.push(reg_cap_node(3, "reg_c", "tool_c"));
        // Only invoke tool_b
        module.nodes.push(inv_tool_node(4, "call_b", "tool_b"));
        let result = tool_binding_check(&module);
        assert!(result.is_ok());
        let warnings = result.unwrap();
        assert_eq!(warnings.len(), 2);
        let names: HashSet<&str> = warnings.iter().map(|w| w.node_name.as_str()).collect();
        assert!(names.contains("reg_a"));
        assert!(names.contains("reg_c"));
    }

    // ---- Mixed scenarios ----

    #[test]
    fn empty_module_passes() {
        let module = empty_module();
        let result = tool_binding_check(&module);
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn non_tool_nodes_ignored() {
        let mut module = empty_module();
        module.nodes.push(AirNode {
            id: 1,
            name: "ask_node".to_string(),
            op: AISOperationType::Ask,
            attributes: HashMap::new(),
        });
        let result = tool_binding_check(&module);
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn e712_and_e713_both_reported() {
        let mut module = empty_module();
        // Invalid handler ID on a registration
        module.nodes.push(reg_cap_with_handler(
            1,
            "reg_bad",
            "bad_tool",
            "not-a-hash",
        ));
        // Unbound capability reference
        module
            .nodes
            .push(inv_tool_node(2, "call_ghost", "ghost_tool"));
        // Also invoke the registered one so it's not unused
        module
            .nodes
            .push(inv_tool_node(3, "call_bad", "bad_tool"));
        let result = tool_binding_check(&module);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("ghost_tool"));
        assert!(err.contains("not-a-hash"));
    }

    // ---- is_valid_handler_id unit tests ----

    #[test]
    fn handler_id_validation() {
        let valid = format!("sha256:{}", "abcdef0123456789".repeat(4));
        assert!(is_valid_handler_id(&valid));
        assert!(!is_valid_handler_id("sha256:short"));
        assert!(!is_valid_handler_id("md5:aaaa"));
        assert!(!is_valid_handler_id(""));
        assert!(!is_valid_handler_id("sha256:"));
        // Exactly 64 hex chars
        assert!(is_valid_handler_id(&format!(
            "sha256:{}",
            "0".repeat(64)
        )));
        // 63 hex chars: too short
        assert!(!is_valid_handler_id(&format!(
            "sha256:{}",
            "0".repeat(63)
        )));
        // 65 hex chars: too long
        assert!(!is_valid_handler_id(&format!(
            "sha256:{}",
            "0".repeat(65)
        )));
    }
}
