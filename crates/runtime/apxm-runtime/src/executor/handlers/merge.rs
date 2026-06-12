//! MERGE operation - Merge multiple values

use super::{ExecutionContext, Node, Result, Value, get_optional_string_attribute};
use apxm_core::constants::graph::attrs as graph_attrs;

pub async fn execute(_ctx: &ExecutionContext, node: &Node, inputs: Vec<Value>) -> Result<Value> {
    let strategy = get_optional_string_attribute(node, graph_attrs::STRATEGY)?
        .unwrap_or_else(|| "array".to_string());

    let separator =
        get_optional_string_attribute(node, graph_attrs::SEPARATOR)?.unwrap_or_default();

    match strategy.as_str() {
        "array" => Ok(Value::Array(inputs)),
        "concat" => {
            // Concatenate string representations with separator
            let parts: Vec<String> = inputs
                .iter()
                .map(|v| {
                    v.as_string().map(|s| s.to_string()).unwrap_or_else(|| {
                        v.to_json()
                            .map(|j| {
                                serde_json::to_string_pretty(&j)
                                    .unwrap_or_else(|_| format!("{:?}", j))
                            })
                            .unwrap_or_else(|_| format!("{:?}", v))
                    })
                })
                .collect();
            Ok(Value::String(parts.join(&separator)))
        }
        "sum" => {
            // Sum numeric values
            let sum = inputs
                .iter()
                .filter_map(|v| v.as_number())
                .map(|n| n.as_f64())
                .sum::<f64>();
            Ok(Value::Number(apxm_core::types::values::Number::Float(sum)))
        }
        "object" => {
            // Merge JSON objects by key (last writer wins)
            let mut merged = std::collections::HashMap::new();
            for val in &inputs {
                if let Value::Object(map) = val {
                    for (k, v) in map {
                        merged.insert(k.clone(), v.clone());
                    }
                }
            }
            Ok(Value::Object(merged))
        }
        _ => Ok(Value::Array(inputs)),
    }
}

