//! Capability metadata and schema definitions

use crate::aam::CapabilityRecord as AamCapabilityRecord;
use apxm_core::types::values::Value;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Metadata describing a capability/tool
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityMetadata {
    /// Unique capability name
    pub name: String,

    /// Human-readable description
    pub description: String,

    /// JSON Schema for input parameters
    pub parameters_schema: serde_json::Value,

    /// Expected return type description
    pub returns: String,

    /// Estimated execution cost (arbitrary units)
    #[serde(default)]
    pub cost_estimate: f64,

    /// Estimated latency in milliseconds
    #[serde(default = "default_latency")]
    pub latency_estimate_ms: u64,

    /// Whether this capability requires authentication
    #[serde(default)]
    pub requires_auth: bool,

    /// Whether invocation must pass the approval gate (consent broker round-trip).
    #[serde(default)]
    pub requires_approval: bool,

    /// Tags for categorization
    #[serde(default)]
    pub tags: Vec<String>,

    /// Tool groups used for least-privilege LLM exposure.
    #[serde(default)]
    pub groups: Vec<String>,

    /// Whether this capability is read-only (safe for full parallel execution).
    /// Write capabilities acquire per-resource locks during parallel dispatch.
    #[serde(default)]
    pub read_only: bool,

    /// Additional metadata
    #[serde(default)]
    pub metadata: HashMap<String, Value>,
}

fn default_latency() -> u64 {
    100
}

impl CapabilityMetadata {
    /// Create new capability metadata with required fields
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        parameters_schema: serde_json::Value,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            parameters_schema,
            returns: "object".to_string(),
            cost_estimate: 0.0,
            latency_estimate_ms: default_latency(),
            requires_auth: false,
            requires_approval: false,
            tags: Vec::new(),
            groups: Vec::new(),
            read_only: false,
            metadata: HashMap::new(),
        }
    }

    /// Set return type description
    pub fn with_returns(mut self, returns: impl Into<String>) -> Self {
        self.returns = returns.into();
        self
    }

    /// Set cost estimate
    pub fn with_cost(mut self, cost: f64) -> Self {
        self.cost_estimate = cost;
        self
    }

    /// Set latency estimate
    pub fn with_latency(mut self, latency_ms: u64) -> Self {
        self.latency_estimate_ms = latency_ms;
        self
    }

    /// Mark as requiring authentication
    pub fn with_auth(mut self) -> Self {
        self.requires_auth = true;
        self
    }

    /// Mark as requiring explicit approval before invocation.
    pub fn with_requires_approval(mut self) -> Self {
        self.requires_approval = true;
        self
    }

    /// Add tags
    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = tags;
        self
    }

    /// Assign tool groups for grouped capability exposure.
    pub fn with_groups(mut self, groups: Vec<String>) -> Self {
        self.groups = groups;
        self
    }

    /// Mark this capability as read-only (safe for full parallel execution).
    pub fn with_read_only(mut self) -> Self {
        self.read_only = true;
        self
    }

    /// Add metadata entry
    pub fn with_metadata(mut self, key: String, value: Value) -> Self {
        self.metadata.insert(key, value);
        self
    }
}

impl From<&CapabilityMetadata> for AamCapabilityRecord {
    fn from(meta: &CapabilityMetadata) -> Self {
        AamCapabilityRecord {
            name: meta.name.clone(),
            description: meta.description.clone(),
            schema: meta.parameters_schema.clone(),
            cost_estimate: meta.cost_estimate,
        }
    }
}
