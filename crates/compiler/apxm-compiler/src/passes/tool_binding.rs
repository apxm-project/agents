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

/// Known builtin capabilities provided by the Rust runtime. Sourced from the
/// canonical list in apxm-core so the compiler and runtime never drift.
const BUILTIN_CAPABILITIES: &[&str] = apxm_core::constants::capabilities::BUILTINS;

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
    known_caps: &HashSet<String>,
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

                // E712: capability must resolve to a REGISTER_CAPABILITY node, a
                // known builtin, or a capability the caller declared available
                // (e.g. provider/pack capabilities the server has registered at
                // runtime — opaque to a standalone compile, declared by the host).
                if !registered.contains_key(cap)
                    && !BUILTIN_CAPABILITIES.contains(&cap)
                    && !known_caps.contains(cap)
                {
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

