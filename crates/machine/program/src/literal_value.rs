//! Bounded materialization of the existing pure-expression literal subset.

use crate::frontend_graph::ValueExpression;
use serde_json::{Map, Value};

impl ValueExpression {
    /// Decode a closed authored JSON literal without an execution environment.
    /// References and projections are intentionally not evaluated here.
    pub fn literal_json(&self) -> Result<Value, &'static str> {
        fn materialize(
            expression: &ValueExpression,
            depth: usize,
            remaining: &mut usize,
        ) -> Result<Value, &'static str> {
            if depth > 32 || *remaining == 0 {
                return Err("default Context exceeds the closed literal size bound");
            }
            *remaining -= 1;
            Ok(match expression {
                ValueExpression::Object { fields } => {
                    let mut object = Map::new();
                    for field in fields {
                        if object.contains_key(&field.name) {
                            return Err("default Context contains a duplicate object key");
                        }
                        object.insert(
                            field.name.clone(),
                            materialize(&field.value, depth + 1, remaining)?,
                        );
                    }
                    Value::Object(object)
                }
                ValueExpression::Array { items } => Value::Array(
                    items
                        .iter()
                        .map(|item| materialize(item, depth + 1, remaining))
                        .collect::<Result<_, _>>()?,
                ),
                ValueExpression::String { value } => Value::String(value.clone()),
                ValueExpression::Integer { value }
                    if value.unsigned_abs() <= 9_007_199_254_740_991 =>
                {
                    Value::from(*value)
                }
                ValueExpression::Boolean { value } => Value::Bool(*value),
                ValueExpression::Null => Value::Null,
                _ => {
                    return Err(
                        "default Context must be a closed JSON literal, not a reference or computed value",
                    );
                }
            })
        }
        materialize(self, 0, &mut 512)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend_graph::ValueField;

    #[test]
    fn defaults_materialize_only_bounded_closed_json_literals() {
        let expression = ValueExpression::Object {
            fields: vec![ValueField {
                name: "messages".into(),
                value: ValueExpression::Array {
                    items: vec![ValueExpression::String {
                        value: "initial".into(),
                    }],
                },
            }],
        };
        assert_eq!(
            expression.literal_json().unwrap(),
            serde_json::json!({"messages":["initial"]})
        );
        assert!(
            ValueExpression::Ssa {
                value_id: "input".into()
            }
            .literal_json()
            .is_err()
        );
        assert!(
            ValueExpression::Context {
                property_path: vec![]
            }
            .literal_json()
            .is_err()
        );
        assert!(
            ValueExpression::Integer { value: i64::MAX }
                .literal_json()
                .is_err()
        );
        let repeated = ValueField {
            name: "same".into(),
            value: ValueExpression::Null,
        };
        assert!(
            ValueExpression::Object {
                fields: vec![repeated.clone(), repeated]
            }
            .literal_json()
            .is_err()
        );
        assert!(
            ValueExpression::Array {
                items: vec![ValueExpression::Null; 512]
            }
            .literal_json()
            .is_err()
        );
    }
}
