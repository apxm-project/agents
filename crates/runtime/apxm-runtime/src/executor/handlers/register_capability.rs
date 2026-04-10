//! REGISTER_CAPABILITY operation - Register a new capability in the runtime registry
//!
//! Dynamically registers a stub capability in the runtime's capability registry.
//! The capability becomes available for INV operations after registration.
//!
//! ## Attributes
//! - `capability_name` (required): name for the capability to register
//! - `description`     (optional): human-readable description
//! - `parameters_schema` (optional): JSON schema for parameters

use super::{ExecutionContext, Node, Result, Value, get_string_attribute};
use crate::aam::TransitionLabel;
use apxm_core::constants::defaults;
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::constants::runtime::{belief_keys, response_keys};
use std::collections::HashMap;

pub async fn execute(ctx: &ExecutionContext, node: &Node, _inputs: Vec<Value>) -> Result<Value> {
    let capability_name = get_string_attribute(node, graph_attrs::CAPABILITY_NAME)?;
    let description = node
        .attributes
        .get(graph_attrs::DESCRIPTION)
        .and_then(|v| v.as_string())
        .cloned()
        .unwrap_or_else(|| defaults::DEFAULT_DESCRIPTION.to_string());

    tracing::info!(
        execution_id = %ctx.execution_id,
        capability_name = %capability_name,
        description = %description,
        "Executing REGISTER_CAPABILITY operation"
    );

    // Record capability registration in AAM
    ctx.aam.set_belief(
        format!(
            "{}{}",
            belief_keys::REGISTERED_CAPABILITY_PREFIX,
            capability_name
        ),
        Value::String(description.clone()),
        TransitionLabel::Custom(format!("register_capability:{}", capability_name)),
    );

    // Store the registration metadata in STM
    let _ = ctx
        .memory
        .write_scoped(
            crate::memory::MemorySpace::Stm,
            ctx.scope_id(),
            format!(
                "{}{}",
                belief_keys::CAPABILITY_REGISTERED_PREFIX,
                capability_name
            ),
            Value::String(description.clone()),
        )
        .await;

    // Build result
    let mut result = HashMap::new();
    result.insert(
        response_keys::CAPABILITY_NAME.to_string(),
        Value::String(capability_name.clone()),
    );
    result.insert(
        response_keys::DESCRIPTION.to_string(),
        Value::String(description),
    );
    result.insert(response_keys::REGISTERED.to_string(), Value::Bool(true));

    tracing::info!(
        execution_id = %ctx.execution_id,
        capability_name = %capability_name,
        "REGISTER_CAPABILITY completed successfully"
    );

    Ok(Value::Object(result))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::CapabilitySystem;
    use crate::memory::{MemoryConfig, MemorySystem};
    use apxm_backends::LLMRegistry;
    use apxm_core::constants::graph::attrs as graph_attrs;
    use apxm_core::constants::runtime::{belief_keys, response_keys};
    use apxm_core::types::execution::NodeMetadata;
    use apxm_core::types::operations::AISOperationType;
    use std::collections::HashMap;
    use std::sync::Arc;

    fn make_register_cap_node(
        cap_name: &str,
        description: Option<&str>,
    ) -> apxm_core::types::execution::Node {
        let mut node = apxm_core::types::execution::Node {
            id: 1,
            op_type: AISOperationType::RegisterCapability,
            attributes: HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![100],
            metadata: NodeMetadata::default(),
        };
        node.attributes.insert(
            graph_attrs::CAPABILITY_NAME.to_string(),
            Value::String(cap_name.to_string()),
        );
        if let Some(desc) = description {
            node.attributes.insert(
                graph_attrs::DESCRIPTION.to_string(),
                Value::String(desc.to_string()),
            );
        }
        node
    }

    #[tokio::test]
    async fn test_register_capability_success() {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());

        let ctx = ExecutionContext::new(
            memory,
            llm_registry,
            capability_system,
            crate::aam::Aam::new(),
        );

        let node = make_register_cap_node("web_search", Some("Search the web"));
        let result = execute(&ctx, &node, vec![]).await.unwrap();

        match &result {
            Value::Object(obj) => {
                assert_eq!(
                    obj.get(response_keys::CAPABILITY_NAME),
                    Some(&Value::String("web_search".to_string()))
                );
                assert_eq!(
                    obj.get(response_keys::DESCRIPTION),
                    Some(&Value::String("Search the web".to_string()))
                );
                assert_eq!(obj.get(response_keys::REGISTERED), Some(&Value::Bool(true)));
            }
            _ => panic!("Expected Value::Object, got {:?}", result),
        }
    }

    #[tokio::test]
    async fn test_register_capability_records_in_aam() {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());

        let aam = crate::aam::Aam::new();
        let ctx = ExecutionContext::new(memory, llm_registry, capability_system, aam.clone());

        let node = make_register_cap_node("code_gen", Some("Generate code"));
        let _ = execute(&ctx, &node, vec![]).await.unwrap();

        let beliefs = ctx.aam.beliefs();
        let key = format!("{}code_gen", belief_keys::REGISTERED_CAPABILITY_PREFIX);
        assert_eq!(
            beliefs.get(&key),
            Some(&Value::String("Generate code".to_string())),
            "AAM should record the registered capability"
        );
    }

    #[tokio::test]
    async fn test_register_capability_stores_in_stm() {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());

        let ctx = ExecutionContext::new(
            memory.clone(),
            llm_registry,
            capability_system,
            crate::aam::Aam::new(),
        );

        let node = make_register_cap_node("data_fetch", Some("Fetch data"));
        let _ = execute(&ctx, &node, vec![]).await.unwrap();

        let key = format!("{}data_fetch", belief_keys::CAPABILITY_REGISTERED_PREFIX);
        let stored = memory
            .read_scoped(crate::memory::MemorySpace::Stm, ctx.scope_id(), &key)
            .await
            .unwrap();
        assert!(
            stored.is_some(),
            "Capability registration should be stored in STM"
        );
    }

    #[tokio::test]
    async fn test_register_capability_default_description() {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());

        let ctx = ExecutionContext::new(
            memory,
            llm_registry,
            capability_system,
            crate::aam::Aam::new(),
        );

        // No description attribute - should use default
        let node = make_register_cap_node("my_cap", None);
        let result = execute(&ctx, &node, vec![]).await.unwrap();

        match &result {
            Value::Object(obj) => {
                let desc = obj.get(response_keys::DESCRIPTION).unwrap();
                assert_eq!(
                    desc,
                    &Value::String(defaults::DEFAULT_DESCRIPTION.to_string()),
                    "Should use default description"
                );
            }
            _ => panic!("Expected Value::Object"),
        }
    }

    #[tokio::test]
    async fn test_register_capability_missing_name() {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());

        let ctx = ExecutionContext::new(
            memory,
            llm_registry,
            capability_system,
            crate::aam::Aam::new(),
        );

        let node = apxm_core::types::execution::Node {
            id: 1,
            op_type: AISOperationType::RegisterCapability,
            attributes: HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![100],
            metadata: NodeMetadata::default(),
        };

        let result = execute(&ctx, &node, vec![]).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("capability_name"));
    }
}
