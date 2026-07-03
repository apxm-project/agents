use serde::{Deserialize, Serialize};

use super::common::{CapabilitySchemaError, validate_non_empty};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityBindingHandler {
    RustExecutor,
    PythonHandler,
    PackHandler,
    McpBridge,
    Builtin,
    Host,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityBinding {
    pub id: String,
    pub handler: CapabilityBindingHandler,
    pub parameters_schema: serde_json::Value,
    pub returns: String,
}

impl CapabilityBinding {
    pub fn validate(&self) -> Result<(), CapabilitySchemaError> {
        validate_non_empty("capability_binding.id", &self.id)?;
        validate_non_empty("capability_binding.returns", &self.returns)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityBindingMetadata {
    pub name: String,
    pub description: String,
    pub parameters_schema: serde_json::Value,
    #[serde(default)]
    pub requires_auth: bool,
    #[serde(default)]
    pub read_only_hint: bool,
}

impl CapabilityBindingMetadata {
    pub fn validate(&self) -> Result<(), CapabilitySchemaError> {
        validate_non_empty("capability_binding.name", &self.name)?;
        validate_non_empty("capability_binding.description", &self.description)
    }
}
