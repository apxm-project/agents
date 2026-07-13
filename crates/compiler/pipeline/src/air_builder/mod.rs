//! AIR Builder — programmatic construction of `.air` (MLIR text) modules.
//!
//! This crate provides the canonical way to construct valid `.air` files from
//! Rust code through a decoupled API that any frontend can use.
//!
//! ```text
//! JSON / TaskDag / programmatic ──→ AirModule ──→ .air text ──→ Pipeline::compile()
//! ```

use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::types::{AISOperationType, DependencyType, Value};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

mod emit;
mod frontend_graph;
mod validate;

#[derive(Debug, Error)]
pub enum AirError {
    #[error("validation error: {0}")]
    Validation(String),
    #[error("emission error: {0}")]
    Emission(String),
}

/// A node in an AIR module (AIS operation with attributes).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AirNode {
    pub id: u64,
    pub name: String,
    pub op: AISOperationType,
    #[serde(default)]
    pub attributes: HashMap<String, Value>,
}

pub use apxm_core::constants::graph::attrs::PromptInputRole;

/// One named LLM input and its rendering role.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PromptInputBinding {
    pub name: String,
    pub role: PromptInputRole,
}

impl PromptInputBinding {
    /// Create a named LLM input binding.
    pub fn new(name: impl Into<String>, role: PromptInputRole) -> Self {
        Self {
            name: name.into(),
            role,
        }
    }
}

/// Apply ordered prompt input bindings to a graph-node attribute map.
pub fn apply_prompt_input_bindings(
    attributes: &mut HashMap<String, Value>,
    bindings: impl IntoIterator<Item = PromptInputBinding>,
) {
    let bindings = bindings.into_iter().collect::<Vec<_>>();
    if bindings.is_empty() {
        attributes.remove(graph_attrs::INPUT_NAMES);
        attributes.remove(graph_attrs::INPUT_ROLES);
        return;
    }

    attributes.insert(
        graph_attrs::INPUT_NAMES.to_string(),
        Value::Array(
            bindings
                .iter()
                .map(|binding| Value::String(binding.name.clone()))
                .collect(),
        ),
    );
    attributes.insert(
        graph_attrs::INPUT_ROLES.to_string(),
        Value::Array(
            bindings
                .iter()
                .map(|binding| Value::String(binding.role.as_str().to_string()))
                .collect(),
        ),
    );
}

impl AirNode {
    /// Attach ordered prompt inputs to an ASK, THINK, or REASON node.
    pub fn with_prompt_inputs(
        mut self,
        bindings: impl IntoIterator<Item = PromptInputBinding>,
    ) -> Self {
        apply_prompt_input_bindings(&mut self.attributes, bindings);
        self
    }
}

/// An edge connecting two nodes.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AirEdge {
    pub from: u64,
    pub to: u64,
    pub dependency: DependencyType,
}

/// A named parameter for a flow function.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AirParam {
    pub name: String,
    pub type_name: String,
}

/// An AIR module — the input representation that gets emitted as `.air` text.
///
/// This is a simple, JSON-serializable struct. Construct it however you like
/// (deserialize from JSON, build programmatically, convert from TaskDag, etc.)
/// then call [`AirModule::to_air()`] to get valid MLIR text.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AirModule {
    pub name: String,
    pub nodes: Vec<AirNode>,
    pub edges: Vec<AirEdge>,
    #[serde(default)]
    pub parameters: Vec<AirParam>,
    #[serde(default)]
    pub metadata: HashMap<String, Value>,
}

impl AirModule {
    /// Validate the module structure (acyclic, unique IDs, required attributes, etc.).
    pub fn validate(&self) -> Result<(), AirError> {
        validate::validate_module(self)
    }

    /// Emit valid `.air` (MLIR text) for this module.
    pub fn to_air(&self) -> Result<String, AirError> {
        emit::emit_air(self)
    }
}

/// A multi-flow AIR program emitted as one MLIR `module { ... }`.
///
/// Each [`AirModule`] is one `func.func` inside the enclosing MLIR module.
/// Frontends that author multiple flows still hand Rust graph modules; the AIR
/// text assembly stays in this crate.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AirProgram {
    #[serde(default)]
    pub modules: Vec<AirModule>,
}

impl AirProgram {
    pub fn new(modules: Vec<AirModule>) -> Self {
        Self { modules }
    }

    /// Validate executable flow ownership before AIR text is assembled.
    pub fn validate(&self) -> Result<(), AirError> {
        validate::validate_program(&self.modules)
    }

    /// Emit valid `.air` (MLIR text) for this multi-flow program.
    pub fn to_air(&self) -> Result<String, AirError> {
        emit::emit_program(&self.modules)
    }
}

/// Builder for constructing an [`AirModule`] programmatically.
pub struct AirModuleBuilder {
    name: String,
    nodes: Vec<AirNode>,
    edges: Vec<AirEdge>,
    parameters: Vec<AirParam>,
    metadata: HashMap<String, Value>,
    next_id: u64,
}

impl AirModuleBuilder {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            nodes: Vec::new(),
            edges: Vec::new(),
            parameters: Vec::new(),
            metadata: HashMap::new(),
            next_id: 1,
        }
    }

    /// Add a node and return its auto-assigned ID.
    pub fn node(
        &mut self,
        name: impl Into<String>,
        op: AISOperationType,
        attributes: HashMap<String, Value>,
    ) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.nodes.push(AirNode {
            id,
            name: name.into(),
            op,
            attributes,
        });
        id
    }

    /// Add a node with a caller-owned stable ID.
    ///
    /// Frontend graph formats already assign stable IDs. Preserve them so
    /// diagnostics and node maps stay attributable across lowering.
    pub fn node_with_id(
        &mut self,
        id: u64,
        name: impl Into<String>,
        op: AISOperationType,
        attributes: HashMap<String, Value>,
    ) -> &mut Self {
        self.next_id = self.next_id.max(id.saturating_add(1));
        self.nodes.push(AirNode {
            id,
            name: name.into(),
            op,
            attributes,
        });
        self
    }

    pub fn edge(&mut self, from: u64, to: u64, dependency: DependencyType) -> &mut Self {
        self.edges.push(AirEdge {
            from,
            to,
            dependency,
        });
        self
    }

    pub fn param(&mut self, name: impl Into<String>, type_name: impl Into<String>) -> &mut Self {
        self.parameters.push(AirParam {
            name: name.into(),
            type_name: type_name.into(),
        });
        self
    }

    pub fn meta(&mut self, key: impl Into<String>, value: Value) -> &mut Self {
        self.metadata.insert(key.into(), value);
        self
    }

    pub fn entry(mut self, is_entry: bool) -> Self {
        self.metadata.insert(
            apxm_core::constants::graph::metadata::IS_ENTRY.to_string(),
            Value::Bool(is_entry),
        );
        self
    }

    pub fn build(self) -> AirModule {
        AirModule {
            name: self.name,
            nodes: self.nodes,
            edges: self.edges,
            parameters: self.parameters,
            metadata: self.metadata,
        }
    }
}

pub use frontend_graph::{
    FrontendEdge, FrontendGraph, FrontendGraphError, FrontendNode, FrontendParameter,
};

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_core::constants::graph::{attrs as graph_attrs, metadata};
    use apxm_core::types::{AISOperationType, Value};
    use std::collections::HashMap;

    fn flow(name: &str, is_entry: Option<bool>) -> AirModule {
        let mut flow_metadata = HashMap::new();
        if let Some(is_entry) = is_entry {
            flow_metadata.insert(metadata::IS_ENTRY.to_string(), Value::Bool(is_entry));
        }
        AirModule {
            name: name.to_string(),
            nodes: vec![AirNode {
                id: 1,
                name: "done".to_string(),
                op: AISOperationType::Return,
                attributes: HashMap::new(),
            }],
            edges: Vec::new(),
            parameters: Vec::new(),
            metadata: flow_metadata,
        }
    }

    #[test]
    fn program_requires_exactly_one_explicit_entry_flow() {
        let missing = AirProgram::new(vec![flow("main", None)]);
        assert!(
            missing
                .validate()
                .expect_err("missing entry metadata must fail")
                .to_string()
                .contains(metadata::IS_ENTRY)
        );

        let multiple = AirProgram::new(vec![flow("main", Some(true)), flow("worker", Some(true))]);
        assert!(
            multiple
                .validate()
                .expect_err("multiple entry flows must fail")
                .to_string()
                .contains("exactly one entry")
        );

        let valid = AirProgram::new(vec![flow("main", Some(true)), flow("worker", Some(false))]);
        valid.validate().expect("one explicit entry flow is valid");
    }

    #[test]
    fn program_rejects_symbols_that_collide_after_sanitization() {
        let program = AirProgram::new(vec![
            flow("main-flow", Some(true)),
            flow("main flow", Some(false)),
        ]);
        assert!(
            program
                .validate()
                .expect_err("sanitized symbol collision must fail")
                .to_string()
                .contains("symbol")
        );
    }

    #[test]
    fn direct_air_prompt_bindings_keep_names_and_roles_in_lockstep() {
        let node = AirNode {
            id: 1,
            name: "ask".to_string(),
            op: AISOperationType::Ask,
            attributes: HashMap::new(),
        }
        .with_prompt_inputs([
            PromptInputBinding::new("question", PromptInputRole::User),
            PromptInputBinding::new("policy", PromptInputRole::System),
            PromptInputBinding::new("dependency", PromptInputRole::DependencyOnly),
            PromptInputBinding::new("tool_result", PromptInputRole::ToolContext),
            PromptInputBinding::new("guard", PromptInputRole::Control),
        ]);

        assert_eq!(
            node.attributes.get(graph_attrs::INPUT_NAMES),
            Some(&Value::Array(vec![
                Value::String("question".to_string()),
                Value::String("policy".to_string()),
                Value::String("dependency".to_string()),
                Value::String("tool_result".to_string()),
                Value::String("guard".to_string()),
            ])),
        );
        assert_eq!(
            node.attributes.get(graph_attrs::INPUT_ROLES),
            Some(&Value::Array(vec![
                Value::String("user".to_string()),
                Value::String("system".to_string()),
                Value::String("dependency_only".to_string()),
                Value::String("tool_context".to_string()),
                Value::String("control".to_string()),
            ])),
        );
    }
}
