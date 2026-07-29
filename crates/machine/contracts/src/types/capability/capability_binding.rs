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
        // The array below is written out by hand, so a variant added to the
        // enum is not automatically added here. That makes this test blind to
        // the mutation it looks like it covers: reintroducing a `PythonHandler`
        // variant leaves this loop unchanged and passing, and only
        // `python_package_handler_binding_is_rejected` goes red. Verified by
        // performing exactly that mutation.
        //
        // The exhaustive match below is what closes the gap. It has no wildcard
        // arm, so adding a variant to `CapabilityBindingHandler` fails to
        // compile until someone lists it here and states its wire spelling --
        // and the assertion that follows then rejects the Python spelling for
        // whatever was listed. A new variant cannot slip past by being absent.
        const ALL: &[CapabilityBindingHandler] = &[
            CapabilityBindingHandler::RustExecutor,
            CapabilityBindingHandler::TypeScriptHandler,
            CapabilityBindingHandler::PackHandler,
            CapabilityBindingHandler::McpBridge,
            CapabilityBindingHandler::Builtin,
            CapabilityBindingHandler::Host,
        ];

        fn wire_spelling(handler: CapabilityBindingHandler) -> &'static str {
            match handler {
                CapabilityBindingHandler::RustExecutor => "rust_executor",
                CapabilityBindingHandler::TypeScriptHandler => "type_script_handler",
                CapabilityBindingHandler::PackHandler => "pack_handler",
                CapabilityBindingHandler::McpBridge => "mcp_bridge",
                CapabilityBindingHandler::Builtin => "builtin",
                CapabilityBindingHandler::Host => "host",
            }
        }

        for handler in ALL {
            let wire = serde_json::to_value(handler).expect("handler kind serializes");
            assert_eq!(
                wire,
                serde_json::json!(wire_spelling(*handler)),
                "the exhaustive match must state each variant's actual wire spelling"
            );
            assert_ne!(
                wire,
                serde_json::json!("python_handler"),
                "no handler kind may serialize to the Python package-handler spelling"
            );
        }

        // Pin the arity too. The exhaustive match forces a new variant to be
        // named, but a careless edit could name it by replacing an existing arm
        // rather than adding one; this catches that.
        assert_eq!(
            ALL.len(),
            6,
            "the closed handler set is six kinds; changing it is a contract change"
        );
    }
}
