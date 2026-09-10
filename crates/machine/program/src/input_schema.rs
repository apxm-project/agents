//! Closed JSON input schemas committed by the compiler with each typed entrypoint.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

/// Maximum nested type depth accepted at the schema boundary.
const MAX_SCHEMA_DEPTH: usize = 32;
/// Maximum combined properties and array elements in one schema.
const MAX_SCHEMA_NODES: usize = 512;

/// JSON types represented by the compiler's supported input contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputSchemaType {
    Object,
    Array,
    String,
    Number,
    Integer,
    Boolean,
    Null,
}

/// A finite JSON schema whose object shapes reject undeclared fields.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntrypointInputSchema {
    #[serde(rename = "type")]
    pub kind: InputSchemaType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub properties: Option<BTreeMap<String, Self>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required: Option<Vec<String>>,
    #[serde(
        rename = "additionalProperties",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub additional_properties: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub items: Option<Box<Self>>,
}

impl EntrypointInputSchema {
    /// Validate the closed schema shape and bounded recursive structure.
    pub fn validate(&self) -> Result<(), &'static str> {
        self.validate_at(0, &mut 0)
    }

    fn validate_at(&self, depth: usize, nodes: &mut usize) -> Result<(), &'static str> {
        *nodes += 1;
        if depth > MAX_SCHEMA_DEPTH || *nodes > MAX_SCHEMA_NODES {
            return Err("input schema exceeds its depth or node limit");
        }
        match self.kind {
            InputSchemaType::Object => {
                let (Some(properties), Some(required), Some(false), None) = (
                    &self.properties,
                    &self.required,
                    self.additional_properties,
                    &self.items,
                ) else {
                    return Err(
                        "object input schema requires properties, required, and additionalProperties:false only",
                    );
                };
                let unique: BTreeSet<_> = required.iter().collect();
                if unique.len() != required.len()
                    || required.iter().any(|name| !properties.contains_key(name))
                {
                    return Err("required input properties must be unique declared names");
                }
                for property in properties.values() {
                    property.validate_at(depth + 1, nodes)?;
                }
            }
            InputSchemaType::Array => {
                if self.properties.is_some()
                    || self.required.is_some()
                    || self.additional_properties.is_some()
                {
                    return Err("array input schema carries object-only fields");
                }
                self.items
                    .as_ref()
                    .ok_or("array input schema requires items")?
                    .validate_at(depth + 1, nodes)?;
            }
            InputSchemaType::String
            | InputSchemaType::Number
            | InputSchemaType::Integer
            | InputSchemaType::Boolean
            | InputSchemaType::Null => {
                if self.properties.is_some()
                    || self.required.is_some()
                    || self.additional_properties.is_some()
                    || self.items.is_some()
                {
                    return Err("scalar input schema carries container fields");
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn accepts_closed_nested_input_and_rejects_widening() {
        let schema = json!({
            "type": "object", "additionalProperties": false,
            "properties": {"reference": {"type": "string"}, "samples": {"type": "array", "items": {"type": "number"}}},
            "required": ["reference"]
        });
        let parsed: EntrypointInputSchema = serde_json::from_value(schema.clone()).unwrap();
        assert!(parsed.validate().is_ok());
        assert_eq!(serde_json::to_value(&parsed).unwrap(), schema);
        for (key, value) in [
            ("additionalProperties", json!(true)),
            ("required", json!(["absent"])),
            ("required", json!(["reference", "reference"])),
            ("items", json!({"type": "string"})),
        ] {
            let mut invalid = schema.clone();
            invalid[key] = value;
            let parsed: EntrypointInputSchema = serde_json::from_value(invalid).unwrap();
            assert!(parsed.validate().is_err());
        }
        let mut unknown = schema;
        unknown["patternProperties"] = json!({});
        assert!(serde_json::from_value::<EntrypointInputSchema>(unknown).is_err());
    }

    #[test]
    fn rejects_incomplete_and_excessively_deep_schemas() {
        for invalid in [
            json!({"type": "object"}),
            json!({"type": "array"}),
            json!({"type": "string", "items": {"type": "null"}}),
        ] {
            let parsed: EntrypointInputSchema = serde_json::from_value(invalid).unwrap();
            assert!(parsed.validate().is_err());
        }
        let mut nested = json!({"type": "string"});
        for _ in 0..=MAX_SCHEMA_DEPTH {
            nested = json!({"type": "array", "items": nested});
        }
        let parsed: EntrypointInputSchema = serde_json::from_value(nested).unwrap();
        assert!(parsed.validate().is_err());
    }
}
