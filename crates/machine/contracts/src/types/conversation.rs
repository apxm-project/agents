//! Canonical conversation turn-input types.
//!
//! The host-facing conversations route accepts a typed turn envelope with a
//! stable `message` field and optional structured `context`.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map as JsonMap, Value as JsonValue};
use thiserror::Error;

use crate::types::values::{Number, Value, ValueError};

/// Structured context attached to one conversation turn.
pub type TurnContext = JsonMap<String, JsonValue>;

/// Canonical host turn-input envelope.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TurnInput {
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<TurnContext>,
}

impl TurnInput {
    pub fn message_only(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            context: None,
        }
    }
}

/// Invalid conversation turn-input data.
#[derive(Debug, Error)]
pub enum TurnInputError {
    #[error("turn input is not representable as JSON: {0}")]
    ValueContract(#[from] ValueError),
    #[error(
        "turn input must be an object with string field 'message' and optional object field 'context': {0}"
    )]
    Envelope(#[from] serde_json::Error),
}

impl TryFrom<Value> for TurnInput {
    type Error = TurnInputError;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        serde_json::from_value(value.to_json()?).map_err(Self::Error::from)
    }
}

impl TryFrom<&Value> for TurnInput {
    type Error = TurnInputError;

    fn try_from(value: &Value) -> Result<Self, Self::Error> {
        Self::try_from(value.clone())
    }
}

impl From<TurnInput> for Value {
    fn from(value: TurnInput) -> Self {
        let mut object = HashMap::from([("message".to_string(), Value::String(value.message))]);
        if let Some(context) = value.context {
            object.insert(
                "context".to_string(),
                Value::Object(
                    context
                        .into_iter()
                        .map(|(key, value)| (key, json_to_runtime_value(value)))
                        .collect(),
                ),
            );
        }
        Value::Object(object)
    }
}

impl From<&TurnInput> for Value {
    fn from(value: &TurnInput) -> Self {
        value.clone().into()
    }
}

fn json_to_runtime_value(value: JsonValue) -> Value {
    match value {
        JsonValue::Null => Value::Null,
        JsonValue::Bool(boolean) => Value::Bool(boolean),
        JsonValue::Number(number) => {
            if let Some(integer) = number.as_i64() {
                Value::Number(Number::Integer(integer))
            } else if let Some(unsigned) = number.as_u64() {
                if i64::try_from(unsigned).is_ok() {
                    Value::Number(Number::Integer(unsigned as i64))
                } else {
                    Value::Number(Number::Float(unsigned as f64))
                }
            } else {
                Value::Number(Number::Float(number.as_f64().unwrap_or(0.0)))
            }
        }
        JsonValue::String(text) => Value::String(text),
        JsonValue::Array(items) => {
            Value::Array(items.into_iter().map(json_to_runtime_value).collect())
        }
        JsonValue::Object(object) => Value::Object(
            object
                .into_iter()
                .map(|(key, value)| (key, json_to_runtime_value(value)))
                .collect(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn bare_string_turn_is_rejected() {
        let error = TurnInput::try_from(Value::String("hello".to_string()))
            .expect_err("turn input requires the typed envelope");
        assert!(error.to_string().contains("turn input must be an object"));
    }

    #[test]
    fn typed_turn_round_trips_through_runtime_value() {
        let turn = TurnInput {
            message: "hello".to_string(),
            context: Some(JsonMap::from_iter([
                ("channel".to_string(), json!("slack")),
                (
                    "metadata".to_string(),
                    json!({"priority": 3, "tags": ["ops", "triage"]}),
                ),
            ])),
        };

        let runtime_value: Value = turn.clone().into();
        let reparsed = TurnInput::try_from(runtime_value).expect("typed turn");

        assert_eq!(reparsed, turn);
    }

    #[test]
    fn missing_message_field_is_rejected() {
        let invalid = Value::Object(HashMap::from([(
            "context".to_string(),
            Value::Object(HashMap::new()),
        )]));

        let error = TurnInput::try_from(invalid).expect_err("message field required");
        assert!(error.to_string().contains("turn input must be"));
    }

    #[test]
    fn non_object_context_is_rejected() {
        let invalid = Value::Object(HashMap::from([
            ("message".to_string(), Value::String("hello".to_string())),
            (
                "context".to_string(),
                Value::String("not-an-object".to_string()),
            ),
        ]));

        let error = TurnInput::try_from(invalid).expect_err("context must be an object");
        assert!(error.to_string().contains("turn input must be"));
    }
}
