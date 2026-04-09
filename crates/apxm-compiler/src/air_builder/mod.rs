//! AIR Builder — programmatic construction of `.air` (MLIR text) modules.
//!
//! This crate provides the canonical way to construct valid `.air` files from
//! Rust code.  It replaces the old `apxm-graph` crate's `to_mlir()` path with
//! a clean, decoupled API that any frontend can use.
//!
//! ```text
//! JSON / TaskDag / programmatic ──→ AirModule ──→ .air text ──→ Pipeline::compile()
//! ```
//!
//! The Python frontend has its own `to_air()` and does not use this crate.

use apxm_core::types::{AISOperationType, DependencyType, Value};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;

mod emit;
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

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_core::constants::graph::attrs as graph_attrs;

    fn make_simple_module(name: &str, id_start: u64) -> AirModule {
        AirModule {
            name: name.to_string(),
            nodes: vec![
                AirNode {
                    id: id_start,
                    name: format!("{}_const", name),
                    op: AISOperationType::ConstStr,
                    attributes: HashMap::from([("value".into(), Value::String("hi".into()))]),
                },
                AirNode {
                    id: id_start + 1,
                    name: format!("{}_ask", name),
                    op: AISOperationType::Ask,
                    attributes: HashMap::from([(
                        graph_attrs::TEMPLATE_STR.into(),
                        Value::String("{0}".into()),
                    )]),
                },
            ],
            edges: vec![AirEdge {
                from: id_start,
                to: id_start + 1,
                dependency: DependencyType::Data,
            }],
            parameters: vec![AirParam {
                name: "input".to_string(),
                type_name: "str".to_string(),
            }],
            metadata: HashMap::new(),
        }
    }

    #[test]
    fn module_roundtrip_air() {
        let module = make_simple_module("test", 1);
        let air = module.to_air().expect("emit to AIR");
        assert!(air.contains("module {"));
        assert!(air.contains("func.func @test"));
    }

    #[test]
    fn module_validates() {
        let module = make_simple_module("valid", 1);
        module.validate().expect("module should be valid");
    }

    #[test]
    fn builder_api() {
        let mut builder = AirModuleBuilder::new("my_flow");
        let c = builder.node(
            "seed",
            AISOperationType::ConstStr,
            HashMap::from([("value".into(), Value::String("hello".into()))]),
        );
        let a = builder.node(
            "ask",
            AISOperationType::Ask,
            HashMap::from([(
                graph_attrs::TEMPLATE_STR.into(),
                Value::String("{0}".into()),
            )]),
        );
        builder.edge(c, a, DependencyType::Data);
        let module = builder.build();

        let air = module.to_air().expect("emit to AIR");
        assert!(air.contains("ais.const_str \"hello\""));
        assert!(air.contains("ais.ask"));
    }
}
