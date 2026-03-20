//! EXC operation - Execute code in sandbox

use super::{
    ExecutionContext, Node, Result, Value, get_optional_string_attribute,
    get_optional_u64_attribute,
};
use crate::sandbox::policy::SandboxPolicy;
use crate::sandbox::process::ProcessSandbox;
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::constants::runtime::belief_keys;

pub async fn execute(ctx: &ExecutionContext, node: &Node, inputs: Vec<Value>) -> Result<Value> {
    // Extract code from 'code' attribute, or fall back to first input
    let code = get_optional_string_attribute(node, graph_attrs::CODE)?
        .or_else(|| {
            inputs
                .first()
                .and_then(|v| v.as_str().map(|s| s.to_string()))
        })
        .ok_or_else(|| apxm_core::error::RuntimeError::Operation {
            op_type: node.op_type,
            message: "EXC requires 'code' attribute or input".to_string(),
        })?;

    let interpreter =
        get_optional_string_attribute(node, "interpreter")?.unwrap_or_else(|| "bash".to_string());

    let timeout_secs = get_optional_u64_attribute(node, "timeout")?.unwrap_or(30);

    let policy = SandboxPolicy {
        timeout: std::time::Duration::from_secs(timeout_secs),
        ..SandboxPolicy::default()
    };

    let sandbox = ProcessSandbox::new(policy);

    // AAM transition
    let transition_label =
        crate::aam::TransitionLabel::operation(node.id, format!("{:?}", node.op_type));
    ctx.aam.set_belief(
        format!("{}{}:started", belief_keys::EXC_PREFIX, node.id),
        Value::String(format!("Executing {} script", interpreter)),
        transition_label.clone(),
    );

    let result = sandbox
        .execute_script(&interpreter, &code)
        .await
        .map_err(|e| apxm_core::error::RuntimeError::Operation {
            op_type: node.op_type,
            message: format!("Sandbox execution failed: {e}"),
        })?;

    if result.timed_out {
        ctx.aam.set_belief(
            format!("{}{}:timed_out", belief_keys::EXC_PREFIX, node.id),
            Value::Bool(true),
            transition_label,
        );
        return Err(apxm_core::error::RuntimeError::Operation {
            op_type: node.op_type,
            message: format!("Script execution timed out after {timeout_secs} seconds"),
        });
    }

    let output = if result.exit_code != 0 {
        format!(
            "Exit code: {}\nStdout: {}\nStderr: {}",
            result.exit_code, result.stdout, result.stderr
        )
    } else {
        result.stdout
    };

    ctx.aam.set_belief(
        format!("{}{}:completed", belief_keys::EXC_PREFIX, node.id),
        Value::String(output.clone()),
        transition_label,
    );

    Ok(Value::String(output))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::CapabilitySystem;
    use crate::memory::{MemoryConfig, MemorySystem};
    use apxm_core::types::operations::AISOperationType;
    use std::sync::Arc;

    fn make_node_with_code(code: &str) -> Node {
        let mut node = Node {
            id: 1,
            op_type: AISOperationType::Exc,
            attributes: std::collections::HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![],
            metadata: apxm_core::types::execution::NodeMetadata::default(),
        };
        node.attributes.insert(
            graph_attrs::CODE.to_string(),
            Value::String(code.to_string()),
        );
        node
    }

    #[tokio::test]
    async fn test_exc_missing_code_errors() {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(apxm_backends::LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());
        let ctx = ExecutionContext::new(
            memory,
            llm_registry,
            capability_system,
            crate::aam::Aam::new(),
        );

        // No 'code' attribute and no inputs => should error
        let node = Node {
            id: 1,
            op_type: AISOperationType::Exc,
            attributes: std::collections::HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![],
            metadata: apxm_core::types::execution::NodeMetadata::default(),
        };
        let result = execute(&ctx, &node, vec![]).await;
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(err_msg.contains("EXC requires"));
    }

    #[tokio::test]
    async fn test_exc_basic_echo() {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(apxm_backends::LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());
        let aam = crate::aam::Aam::new();
        let ctx = ExecutionContext::new(memory, llm_registry, capability_system, aam.clone());

        let node = make_node_with_code("echo hello");
        let result = execute(&ctx, &node, vec![]).await.unwrap();
        // Should contain "hello" in the output
        if let Value::String(s) = &result {
            assert!(
                s.contains("hello"),
                "expected 'hello' in output, got: {}",
                s
            );
        } else {
            panic!("Expected string result from EXC");
        }

        // Check that the AAM recorded the completion
        let beliefs = aam.beliefs();
        let completed_key = format!("{}{}:completed", belief_keys::EXC_PREFIX, node.id);
        assert!(beliefs.contains_key(&completed_key));
    }

    #[tokio::test]
    async fn test_exc_code_from_input() {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(apxm_backends::LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());
        let ctx = ExecutionContext::new(
            memory,
            llm_registry,
            capability_system,
            crate::aam::Aam::new(),
        );

        // No code attribute, but first input provides the code
        let node = Node {
            id: 2,
            op_type: AISOperationType::Exc,
            attributes: std::collections::HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![],
            metadata: apxm_core::types::execution::NodeMetadata::default(),
        };
        let result = execute(
            &ctx,
            &node,
            vec![Value::String("echo from_input".to_string())],
        )
        .await
        .unwrap();
        if let Value::String(s) = &result {
            assert!(
                s.contains("from_input"),
                "expected 'from_input' in output, got: {}",
                s
            );
        } else {
            panic!("Expected string result from EXC");
        }
    }
}
