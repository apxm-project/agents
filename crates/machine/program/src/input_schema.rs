//! Closed JSON input schemas committed by the compiler with each typed entrypoint.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

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
    /// Content address of the validated schema with recursively sorted object keys.
    pub fn canonical_digest(&self) -> Result<String, &'static str> {
        self.validate()?;
        fn sorted(value: serde_json::Value) -> serde_json::Value {
            match value {
                serde_json::Value::Object(fields) => {
                    let fields: BTreeMap<_, _> = fields.into_iter().collect();
                    serde_json::Value::Object(
                        fields
                            .into_iter()
                            .map(|(key, value)| (key, sorted(value)))
                            .collect(),
                    )
                }
                serde_json::Value::Array(items) => {
                    serde_json::Value::Array(items.into_iter().map(sorted).collect())
                }
                other => other,
            }
        }
        let value = serde_json::to_value(self).map_err(|_| "schema serialization failed")?;
        let bytes =
            serde_json::to_vec(&sorted(value)).map_err(|_| "schema serialization failed")?;
        Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
    }

    /// Check bounded JSON data without coercion or undeclared object fields.
    pub fn validate_value(&self, value: &serde_json::Value) -> Result<(), &'static str> {
        self.validate()?;
        self.validate_value_at(value, 0, &mut 0)
    }

    fn validate_value_at(
        &self,
        value: &serde_json::Value,
        depth: usize,
        nodes: &mut usize,
    ) -> Result<(), &'static str> {
        *nodes += 1;
        if depth > MAX_SCHEMA_DEPTH || *nodes > MAX_SCHEMA_NODES {
            return Err("JSON value exceeds its depth or node limit");
        }
        let matches = match self.kind {
            InputSchemaType::Object => {
                let object = value.as_object().ok_or("expected JSON object")?;
                let properties = self.properties.as_ref().ok_or("missing object schema")?;
                if self
                    .required
                    .as_ref()
                    .ok_or("missing required fields")?
                    .iter()
                    .any(|key| !object.contains_key(key))
                {
                    return Err("missing required JSON property");
                }
                for (key, value) in object {
                    properties
                        .get(key)
                        .ok_or("undeclared JSON property")?
                        .validate_value_at(value, depth + 1, nodes)?;
                }
                true
            }
            InputSchemaType::Array => {
                let items = value.as_array().ok_or("expected JSON array")?;
                let schema = self.items.as_ref().ok_or("missing array schema")?;
                for item in items {
                    schema.validate_value_at(item, depth + 1, nodes)?;
                }
                true
            }
            InputSchemaType::String => value.is_string(),
            InputSchemaType::Boolean => value.is_boolean(),
            InputSchemaType::Null => value.is_null(),
            InputSchemaType::Number | InputSchemaType::Integer => {
                value.as_f64().is_some_and(|number| {
                    number.is_finite()
                        && (self.kind != InputSchemaType::Integer || number.fract() == 0.0)
                        && (number.fract() != 0.0 || number.abs() <= 9_007_199_254_740_991.0)
                })
            }
        };
        if matches {
            Ok(())
        } else {
            Err("JSON value does not match its declared type")
        }
    }

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
    fn schema_digest_and_payload_validation_are_shared_and_closed() {
        let schema: EntrypointInputSchema =
            serde_json::from_value(json!({"type":"string"})).unwrap();
        assert_eq!(
            schema.canonical_digest().unwrap(),
            "sha256:00404e686415370f1711c4d7acfa2905444d3cf23cef2e10c47d445ebe690f96"
        );
        assert!(schema.validate_value(&json!("answer")).is_ok());
        assert!(schema.validate_value(&json!(1)).is_err());
        let schema: EntrypointInputSchema = serde_json::from_value(json!({
            "type":"object", "properties":{"approved":{"type":"boolean"},"items":{"type":"array","items":{"type":"integer"}}},
            "required":["approved"],"additionalProperties":false
        })).unwrap();
        assert!(
            schema
                .validate_value(&json!({"approved":true,"items":[0,2]}))
                .is_ok()
        );
        for invalid in [
            json!({}),
            json!({"approved":"true"}),
            json!({"approved":true,"extra":1}),
            json!({"approved":true,"items":[1.5]}),
            json!({"approved":true,"items":[9007199254740992_u64]}),
        ] {
            assert!(schema.validate_value(&invalid).is_err());
        }
        let mut reordered = serde_json::to_value(&schema).unwrap();
        let object = reordered.as_object_mut().unwrap();
        let properties = object.remove("properties").unwrap();
        object.insert("properties".into(), properties);
        let reordered: EntrypointInputSchema = serde_json::from_value(reordered).unwrap();
        assert_eq!(
            schema.canonical_digest().unwrap(),
            reordered.canonical_digest().unwrap()
        );
    }

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
