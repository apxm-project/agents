use serde::{Deserialize, Serialize};

use super::common::{CapabilitySchemaError, validate_non_empty};

/// The closed set of handler kinds a capability binding may declare.
///
/// Package-local handlers are TypeScript-only: Python authoring exposes no
/// package-local handler API, so no Python package-handler kind is
/// representable here or admissible on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityBindingHandler {
    RustExecutor,
    TypeScriptHandler,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn binding_json(handler: &str) -> serde_json::Value {
        serde_json::json!({
            "id": "files.read",
            "handler": handler,
            "parameters_schema": {"type": "object"},
            "returns": "json",
        })
    }

    #[test]
    fn typescript_package_handler_binding_is_admitted() {
        let binding: CapabilityBinding =
            serde_json::from_value(binding_json("type_script_handler"))
                .expect("the TypeScript package-handler kind is admissible");
        assert_eq!(binding.handler, CapabilityBindingHandler::TypeScriptHandler);
        binding.validate().expect("well-formed binding validates");
    }

    #[test]
    fn python_package_handler_binding_is_rejected() {
        let error = serde_json::from_value::<CapabilityBinding>(binding_json("python_handler"))
            .expect_err("a Python package-handler binding must not deserialize");
        assert!(
            error
                .to_string()
                .contains("unknown variant `python_handler`"),
            "expected an unknown-variant rejection, got: {error}"
        );
    }

    #[test]
    fn python_package_handler_is_not_in_the_closed_handler_set() {
        for handler in [
            CapabilityBindingHandler::RustExecutor,
            CapabilityBindingHandler::TypeScriptHandler,
            CapabilityBindingHandler::PackHandler,
            CapabilityBindingHandler::McpBridge,
            CapabilityBindingHandler::Builtin,
            CapabilityBindingHandler::Host,
        ] {
            let wire = serde_json::to_value(handler).expect("handler kind serializes");
            assert_ne!(
                wire,
                serde_json::json!("python_handler"),
                "no handler kind may serialize to the Python package-handler spelling"
            );
        }
    }
}
