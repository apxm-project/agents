use serde::{Deserialize, Serialize};

use super::common::{validate_non_empty, CapabilitySchemaError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolBindingHandler {
    RustExecutor,
    PythonTool,
    PackHandler,
    McpBridge,
    Builtin,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolBinding {
    pub id: String,
    pub handler: ToolBindingHandler,
    pub parameters_schema: serde_json::Value,
    pub returns: String,
}

impl ToolBinding {
    pub fn validate(&self) -> Result<(), CapabilitySchemaError> {
        validate_non_empty("tool_binding.id", &self.id)?;
        validate_non_empty("tool_binding.returns", &self.returns)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolBindingMetadata {
    pub name: String,
    pub description: String,
    pub parameters_schema: serde_json::Value,
    #[serde(default)]
    pub requires_auth: bool,
    #[serde(default)]
    pub read_only_hint: bool,
}

impl ToolBindingMetadata {
    pub fn validate(&self) -> Result<(), CapabilitySchemaError> {
        validate_non_empty("tool_binding.name", &self.name)?;
        validate_non_empty("tool_binding.description", &self.description)
    }
}
