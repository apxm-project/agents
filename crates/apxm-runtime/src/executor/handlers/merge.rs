//! MERGE operation - Merge multiple values

use super::{ExecutionContext, Node, Result, Value, get_optional_string_attribute};
use apxm_core::constants::graph::attrs as graph_attrs;

pub async fn execute(_ctx: &ExecutionContext, node: &Node, inputs: Vec<Value>) -> Result<Value> {
    let strategy = get_optional_string_attribute(node, graph_attrs::STRATEGY)?
        .unwrap_or_else(|| "array".to_string());

    // Optional separator for concat (default: empty string for backward compat)
    let separator = get_optional_string_attribute(node, graph_attrs::SEPARATOR)?
        .unwrap_or_default();

    match strategy.as_str() {
        "array" => Ok(Value::Array(inputs)),
        "concat" => {
            // Concatenate string representations with separator
            let parts: Vec<String> = inputs
                .iter()
                .map(|v| {
                    v.as_string()
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| {
                            v.to_json()
                                .map(|j| serde_json::to_string_pretty(&j).unwrap_or_else(|_| format!("{:?}", j)))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::CapabilitySystem;
    use crate::memory::{MemoryConfig, MemorySystem};
    use apxm_core::types::operations::AISOperationType;
    use apxm_core::types::values::Number;
    use std::sync::Arc;

    async fn test_ctx() -> ExecutionContext {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        ExecutionContext::new(
            memory,
            Arc::new(apxm_backends::LLMRegistry::new()),
            Arc::new(CapabilitySystem::new()),
            crate::aam::Aam::new(),
        )
    }

    fn merge_node(strategy: Option<&str>) -> Node {
        let mut node = Node::new(1, AISOperationType::Merge);
        if let Some(s) = strategy {
            node.attributes.insert(
                graph_attrs::STRATEGY.to_string(),
                Value::String(s.to_string()),
            );
        }
        node
    }

    #[tokio::test]
    async fn test_merge_array_default() {
        let ctx = test_ctx().await;
        let node = merge_node(None);
        let inputs = vec![
            Value::String("a".to_string()),
            Value::String("b".to_string()),
        ];
        let result = execute(&ctx, &node, inputs.clone()).await.unwrap();
        assert_eq!(result, Value::Array(inputs));
    }

    #[tokio::test]
    async fn test_merge_array_explicit() {
        let ctx = test_ctx().await;
        let node = merge_node(Some("array"));
        let inputs = vec![Value::Bool(true), Value::Null];
        let result = execute(&ctx, &node, inputs.clone()).await.unwrap();
        assert_eq!(result, Value::Array(inputs));
    }

    #[tokio::test]
    async fn test_merge_concat() {
        let ctx = test_ctx().await;
        let node = merge_node(Some("concat"));
        let inputs = vec![
            Value::String("hello".to_string()),
            Value::String(" world".to_string()),
        ];
        let result = execute(&ctx, &node, inputs).await.unwrap();
        assert_eq!(result, Value::String("hello world".to_string()));
    }

    #[tokio::test]
    async fn test_merge_sum() {
        let ctx = test_ctx().await;
        let node = merge_node(Some("sum"));
        let inputs = vec![
            Value::Number(Number::Integer(10)),
            Value::Number(Number::Integer(20)),
            Value::Number(Number::Float(0.5)),
        ];
        let result = execute(&ctx, &node, inputs).await.unwrap();
        match result {
            Value::Number(Number::Float(f)) => assert!((f - 30.5).abs() < f64::EPSILON),
            other => panic!("Expected float number, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_merge_unknown_strategy_falls_back_to_array() {
        let ctx = test_ctx().await;
        let node = merge_node(Some("unknown"));
        let inputs = vec![Value::Bool(false)];
        let result = execute(&ctx, &node, inputs.clone()).await.unwrap();
        assert_eq!(result, Value::Array(inputs));
    }

    #[tokio::test]
    async fn test_merge_empty_inputs() {
        let ctx = test_ctx().await;
        let node = merge_node(None);
        let result = execute(&ctx, &node, vec![]).await.unwrap();
        assert_eq!(result, Value::Array(vec![]));
    }
}
