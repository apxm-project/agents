//! Tool Binding Check Pass
//!
//! Validates that every `INV_TOOL.capability` resolves to either an upstream
//! `REGISTER_CAPABILITY` in the same module or a known Rust builtin capability.
//! Also validates `python_handler_id` format and warns on unused capabilities
//! and orphan Python tools.
//!
//! Diagnostics:
//! - E712: unbound capability (INV_TOOL references unknown tool)
//! - E713: invalid `python_handler_id` format (must be `sha256:<hex64>`)
//! - W721: unused capability (REGISTER_CAPABILITY never invoked)
//! - W723: orphan Python tool (manifest entry with no matching REGISTER_CAPABILITY)

use crate::air_builder::AirModule;
use apxm_core::constants::graph::attrs;
use apxm_core::error::compiler::{CompilerError, Result};
use apxm_core::error::span::Span;
use apxm_core::error::{Error, ErrorCode};
use apxm_core::types::AISOperationType;
use apxm_core::types::execution::ExecutionDag;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};

/// Lightweight manifest entry for a Python `@tool`-decorated function.
///
/// Mirrors the fields of `ToolDescriptor` (from `apxm-runtime`) that are
/// relevant to compile-time orphan detection. The compiler crate does not
/// depend on `apxm-runtime`, so we keep a minimal copy here.
#[derive(Debug, Clone, Deserialize)]
pub struct PythonToolManifestEntry {
    /// Unique handler identifier (`sha256:<hash>`).
    pub handler_id: String,
    /// Python module path (e.g. `myapp.tools`).
    pub module: String,
    /// Qualified name within the module (e.g. `add`).
    pub qualname: String,
}

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
/// When `manifest` is provided, the pass also checks for orphan Python tools:
/// manifest entries whose `handler_id` is not referenced by any
/// `REGISTER_CAPABILITY` node via the `python_handler_id` attribute.
///
/// Returns `Ok(diagnostics)` where diagnostics may contain warnings,
/// or `Err(CompilerError)` if hard errors (E712, E713) are found.
pub fn tool_binding_check(
    module: &AirModule,
    manifest: Option<&[PythonToolManifestEntry]>,
) -> Result<Vec<ToolBindingDiagnostic>> {
    let mut errors: Vec<ToolBindingDiagnostic> = Vec::new();
    let mut warnings: Vec<ToolBindingDiagnostic> = Vec::new();

    // Collect all registered capability names from REGISTER_CAPABILITY nodes.
    let mut registered: HashMap<String, Vec<String>> = HashMap::new();
    for node in &module.nodes {
        if node.op == AISOperationType::RegisterCapability {
            if let Some(name) = node
                .attributes
                .get(attrs::CAPABILITY_NAME)
                .and_then(|v| v.as_str())
            {
                registered
                    .entry(name.to_string())
                    .or_default()
                    .push(node.name.clone());
            }
        }
    }

    // Collect all invoked capability names from INV_TOOL nodes and ASK tool lists.
    let mut invoked: HashSet<String> = HashSet::new();
    for node in &module.nodes {
        if node.op == AISOperationType::InvTool {
            if let Some(cap) = node
                .attributes
                .get(attrs::CAPABILITY)
                .and_then(|v| v.as_str())
            {
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
        if node.op == AISOperationType::Ask
            && let Some(tools) = node.attributes.get(attrs::TOOLS).and_then(|v| v.as_array())
        {
            invoked.extend(
                tools
                    .iter()
                    .filter_map(|tool| tool.as_str().map(ToString::to_string)),
            );
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
                         INV_TOOL node or ASK tools list",
                        cap_name, reg_node,
                    ),
                    node_name: reg_node.clone(),
                });
            }
        }
    }

    // W723: orphan Python tool — manifest entry whose handler_id is not
    // referenced by any REGISTER_CAPABILITY node's python_handler_id attr.
    if let Some(entries) = manifest {
        let referenced_handler_ids: HashSet<&str> = module
            .nodes
            .iter()
            .filter(|n| n.op == AISOperationType::RegisterCapability)
            .filter_map(|n| {
                n.attributes
                    .get(attrs::PYTHON_HANDLER_ID)
                    .and_then(|v| v.as_str())
            })
            .collect();

        for entry in entries {
            if !referenced_handler_ids.contains(entry.handler_id.as_str()) {
                warnings.push(ToolBindingDiagnostic {
                    code: ErrorCode::OrphanPythonTool,
                    message: format!(
                        "@tool '{}' (module '{}') is declared in the Python manifest \
                         but no REGISTER_CAPABILITY node references its handler_id",
                        entry.qualname, entry.module,
                    ),
                    node_name: entry.qualname.clone(),
                });
            }
        }
    }

    // Schema-vs-signature drift check not yet implemented; requires Python handler signatures in compiler metadata.

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

/// Same checks as [`tool_binding_check`] but on a post-MLIR [`ExecutionDag`].
///
/// Returns `Ok(diagnostics)` containing any W721/W723 warnings, or `Err` on
/// hard errors (E712, E713).
pub fn tool_binding_check_dag(
    dag: &ExecutionDag,
    manifest: Option<&[PythonToolManifestEntry]>,
) -> Result<Vec<ToolBindingDiagnostic>> {
    let mut errors: Vec<ToolBindingDiagnostic> = Vec::new();
    let mut warnings: Vec<ToolBindingDiagnostic> = Vec::new();

    // Collect all registered capability names from REGISTER_CAPABILITY nodes.
    let mut registered: HashMap<String, Vec<String>> = HashMap::new();
    for node in &dag.nodes {
        if node.op_type == AISOperationType::RegisterCapability {
            if let Some(name) = node
                .attributes
                .get(attrs::CAPABILITY_NAME)
                .and_then(|v| v.as_str())
            {
                registered
                    .entry(name.to_string())
                    .or_default()
                    .push(format!("#{}", node.id));
            }
        }
    }

    // Collect all invoked capability names from INV_TOOL nodes and ASK tool lists.
    let mut invoked: HashSet<String> = HashSet::new();
    for node in &dag.nodes {
        if node.op_type == AISOperationType::InvTool {
            if let Some(cap) = node
                .attributes
                .get(attrs::CAPABILITY)
                .and_then(|v| v.as_str())
            {
                invoked.insert(cap.to_string());

                // E712: capability must resolve to REGISTER_CAPABILITY or builtin.
                if !registered.contains_key(cap) && !BUILTIN_CAPABILITIES.contains(&cap) {
                    errors.push(ToolBindingDiagnostic {
                        code: ErrorCode::UnboundCapability,
                        message: format!(
                            "INV_TOOL node #{} references capability '{}' which is not \
                             registered by any REGISTER_CAPABILITY node and is not a \
                             known builtin ({})",
                            node.id,
                            cap,
                            BUILTIN_CAPABILITIES.join(", "),
                        ),
                        node_name: format!("#{}", node.id),
                    });
                }
            }
        }
        if node.op_type == AISOperationType::Ask
            && let Some(tools) = node.attributes.get(attrs::TOOLS).and_then(|v| v.as_array())
        {
            invoked.extend(
                tools
                    .iter()
                    .filter_map(|tool| tool.as_str().map(ToString::to_string)),
            );
        }
    }

    // E713: validate python_handler_id format on REGISTER_CAPABILITY nodes.
    for node in &dag.nodes {
        if node.op_type == AISOperationType::RegisterCapability {
            if let Some(handler_id) = node
                .attributes
                .get(attrs::PYTHON_HANDLER_ID)
                .and_then(|v| v.as_str())
            {
                if !is_valid_handler_id(handler_id) {
                    errors.push(ToolBindingDiagnostic {
                        code: ErrorCode::InvalidHandlerId,
                        message: format!(
                            "REGISTER_CAPABILITY node #{} has invalid python_handler_id \
                             '{}'; expected format: sha256:<64 hex chars>",
                            node.id, handler_id,
                        ),
                        node_name: format!("#{}", node.id),
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
                        "REGISTER_CAPABILITY '{}' in node {} is never invoked by any \
                         INV_TOOL node or ASK tools list",
                        cap_name, reg_node,
                    ),
                    node_name: reg_node.clone(),
                });
            }
        }
    }

    // W723: orphan Python tool — manifest entry whose handler_id is not
    // referenced by any REGISTER_CAPABILITY node's python_handler_id attr.
    if let Some(entries) = manifest {
        let referenced_handler_ids: HashSet<&str> = dag
            .nodes
            .iter()
            .filter(|n| n.op_type == AISOperationType::RegisterCapability)
            .filter_map(|n| {
                n.attributes
                    .get(attrs::PYTHON_HANDLER_ID)
                    .and_then(|v| v.as_str())
            })
            .collect();

        for entry in entries {
            if !referenced_handler_ids.contains(entry.handler_id.as_str()) {
                warnings.push(ToolBindingDiagnostic {
                    code: ErrorCode::OrphanPythonTool,
                    message: format!(
                        "@tool '{}' (module '{}') is declared in the Python manifest \
                         but no REGISTER_CAPABILITY node references its handler_id",
                        entry.qualname, entry.module,
                    ),
                    node_name: entry.qualname.clone(),
                });
            }
        }
    }

    if !errors.is_empty() {
        let error_messages: Vec<String> = errors.iter().map(|e| e.message.clone()).collect();
        let combined = format!(
            "Tool binding check failed:\n\n{}",
            error_messages.join("\n"),
        );

        return Err(CompilerError::Verification(Box::new(Error::new(
            errors[0].code,
            combined,
            Span::new("<artifact>".to_string(), 1, 1, 0),
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
        let result = tool_binding_check(&module, None);
        assert!(result.is_ok());
    }

    #[test]
    fn e712_inv_tool_references_builtin_capability() {
        let mut module = empty_module();
        for (i, builtin) in BUILTIN_CAPABILITIES.iter().enumerate() {
            module.nodes.push(inv_tool_node(
                (i + 1) as u64,
                &format!("call_{builtin}"),
                builtin,
            ));
        }
        let result = tool_binding_check(&module, None);
        assert!(result.is_ok());
    }

    #[test]
    fn e712_inv_tool_references_unknown_capability() {
        let mut module = empty_module();
        module
            .nodes
            .push(inv_tool_node(1, "call_missing", "nonexistent_tool"));
        let result = tool_binding_check(&module, None);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("nonexistent_tool"));
        assert!(err.contains("call_missing"));
    }

    #[test]
    fn e712_multiple_unbound_capabilities_all_reported() {
        let mut module = empty_module();
        module.nodes.push(inv_tool_node(1, "call_a", "missing_a"));
        module.nodes.push(inv_tool_node(2, "call_b", "missing_b"));
        let result = tool_binding_check(&module, None);
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
        module
            .nodes
            .push(reg_cap_with_handler(1, "reg_add", "add", &valid_hash));
        module.nodes.push(inv_tool_node(2, "call_add", "add"));
        let result = tool_binding_check(&module, None);
        assert!(result.is_ok());
    }

    #[test]
    fn e713_invalid_handler_id_too_short() {
        let mut module = empty_module();
        module
            .nodes
            .push(reg_cap_with_handler(1, "reg_add", "add", "sha256:abcd"));
        module.nodes.push(inv_tool_node(2, "call_add", "add"));
        let result = tool_binding_check(&module, None);
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
        let result = tool_binding_check(&module, None);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("md5:"));
    }

    #[test]
    fn e713_invalid_handler_id_non_hex() {
        let mut module = empty_module();
        let bad_hex = format!("sha256:{}z", "a".repeat(63));
        module
            .nodes
            .push(reg_cap_with_handler(1, "reg_add", "add", &bad_hex));
        module.nodes.push(inv_tool_node(2, "call_add", "add"));
        let result = tool_binding_check(&module, None);
        assert!(result.is_err());
    }

    #[test]
    fn e713_no_handler_id_is_fine() {
        // REGISTER_CAPABILITY without python_handler_id is valid (Rust builtin capability)
        let mut module = empty_module();
        module.nodes.push(reg_cap_node(1, "reg_tool", "my_tool"));
        module.nodes.push(inv_tool_node(2, "call_tool", "my_tool"));
        let result = tool_binding_check(&module, None);
        assert!(result.is_ok());
    }

    // ---- W721: unused capability ----

    #[test]
    fn w721_unused_capability_emits_warning() {
        let mut module = empty_module();
        module
            .nodes
            .push(reg_cap_node(1, "reg_unused", "unused_tool"));
        let result = tool_binding_check(&module, None);
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
        let result = tool_binding_check(&module, None);
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
        let result = tool_binding_check(&module, None);
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
        let result = tool_binding_check(&module, None);
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
        let result = tool_binding_check(&module, None);
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn e712_and_e713_both_reported() {
        let mut module = empty_module();
        // Invalid handler ID on a registration
        module
            .nodes
            .push(reg_cap_with_handler(1, "reg_bad", "bad_tool", "not-a-hash"));
        // Unbound capability reference
        module
            .nodes
            .push(inv_tool_node(2, "call_ghost", "ghost_tool"));
        // Also invoke the registered one so it's not unused
        module.nodes.push(inv_tool_node(3, "call_bad", "bad_tool"));
        let result = tool_binding_check(&module, None);
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
        assert!(is_valid_handler_id(&format!("sha256:{}", "0".repeat(64))));
        // 63 hex chars: too short
        assert!(!is_valid_handler_id(&format!("sha256:{}", "0".repeat(63))));
        // 65 hex chars: too long
        assert!(!is_valid_handler_id(&format!("sha256:{}", "0".repeat(65))));
    }

    // ---- W723: orphan Python tool ----

    fn manifest_entry(handler_id: &str, module: &str, qualname: &str) -> PythonToolManifestEntry {
        PythonToolManifestEntry {
            handler_id: handler_id.to_string(),
            module: module.to_string(),
            qualname: qualname.to_string(),
        }
    }

    #[test]
    fn w723_orphan_tool_no_python_ops() {
        // Module with no REGISTER_CAPABILITY nodes + manifest with one tool
        // => exactly one W723 warning.
        let module = empty_module();
        let handler = format!("sha256:{}", "a".repeat(64));
        let manifest = vec![manifest_entry(&handler, "myapp.tools", "add")];
        let result = tool_binding_check(&module, Some(&manifest));
        assert!(result.is_ok());
        let warnings = result.unwrap();
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].code, ErrorCode::OrphanPythonTool);
        assert!(warnings[0].message.contains("add"));
        assert!(warnings[0].message.contains("myapp.tools"));
    }

    #[test]
    fn w723_no_orphan_when_handler_referenced() {
        // Module with REGISTER_CAPABILITY referencing the tool's handler_id
        // => zero W723 warnings.
        let handler = format!("sha256:{}", "a".repeat(64));
        let mut module = empty_module();
        module
            .nodes
            .push(reg_cap_with_handler(1, "reg_add", "add", &handler));
        module.nodes.push(inv_tool_node(2, "call_add", "add"));
        let manifest = vec![manifest_entry(&handler, "myapp.tools", "add")];
        let result = tool_binding_check(&module, Some(&manifest));
        assert!(result.is_ok());
        let warnings = result.unwrap();
        assert!(
            warnings.is_empty(),
            "Expected no warnings but got: {:?}",
            warnings
        );
    }

    #[test]
    fn w723_no_manifest_no_orphan_warnings() {
        // No manifest provided => no W723 warnings even with empty module.
        let module = empty_module();
        let result = tool_binding_check(&module, None);
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn w723_multiple_orphan_tools() {
        let module = empty_module();
        let h1 = format!("sha256:{}", "a".repeat(64));
        let h2 = format!("sha256:{}", "b".repeat(64));
        let manifest = vec![
            manifest_entry(&h1, "myapp.tools", "add"),
            manifest_entry(&h2, "myapp.tools", "multiply"),
        ];
        let result = tool_binding_check(&module, Some(&manifest));
        assert!(result.is_ok());
        let warnings = result.unwrap();
        assert_eq!(warnings.len(), 2);
        assert!(
            warnings
                .iter()
                .all(|w| w.code == ErrorCode::OrphanPythonTool)
        );
        let names: HashSet<&str> = warnings.iter().map(|w| w.node_name.as_str()).collect();
        assert!(names.contains("add"));
        assert!(names.contains("multiply"));
    }
}
