//! Capability executor trait and execution infrastructure

use super::metadata::RuntimeCapability;
use apxm_capability_iface::CapabilityInvocation;
use apxm_capability_iface::sandbox::{ExecRequest, ExecResult};
use apxm_core::events::payload::CapabilityEffectReceiptPayload;
use apxm_core::{error::RuntimeError, types::values::Value};
use async_trait::async_trait;
use std::collections::HashMap;
use std::fmt::Write as _;

/// Result type for capability operations
pub type CapabilityResult<T> = Result<T, RuntimeError>;

/// A capability value plus optional evidence for a durably committed effect.
///
/// Implementations attach a receipt only after their owning boundary has
/// persisted verified private evidence. The runtime emits the receipt without
/// exposing the private preparation, approval, or signature material.
#[derive(Debug, Clone)]
pub struct CapabilityExecutionResult {
    pub value: Value,
    pub effect_receipt: Option<CapabilityEffectReceiptPayload>,
}

impl CapabilityExecutionResult {
    pub fn new(value: Value) -> Self {
        Self {
            value,
            effect_receipt: None,
        }
    }

    pub fn with_effect_receipt(
        value: Value,
        effect_receipt: CapabilityEffectReceiptPayload,
    ) -> Self {
        Self {
            value,
            effect_receipt: Some(effect_receipt),
        }
    }
}

/// Trait for capability implementations
///
/// Capabilities are executable tools/functions that can be invoked
/// by the APxM runtime. Each capability must provide metadata and
/// an async execution method.
#[async_trait]
pub trait CapabilityExecutor: Send + Sync {
    /// Execute the capability with given arguments
    ///
    /// # Arguments
    ///
    /// * `args` - Input arguments as key-value pairs
    ///
    /// # Returns
    ///
    /// Result value from capability execution
    ///
    /// # Errors
    ///
    /// Returns RuntimeError::Capability if execution fails
    async fn execute(&self, args: HashMap<String, Value>) -> CapabilityResult<Value>;

    /// Execute with trusted invocation identity when the caller has it.
    ///
    /// Existing native capability implementations retain their `execute`
    /// contract; only durable external effect executors override this method.
    async fn execute_with_invocation(
        &self,
        args: HashMap<String, Value>,
        invocation: Option<&CapabilityInvocation>,
    ) -> CapabilityResult<Value> {
        let _ = invocation;
        self.execute(args).await
    }

    /// Execute with trusted invocation identity and return a receipt when the
    /// implementation has already committed verified durable evidence.
    async fn execute_with_effect_receipt(
        &self,
        args: HashMap<String, Value>,
        invocation: Option<&CapabilityInvocation>,
    ) -> CapabilityResult<CapabilityExecutionResult> {
        self.execute_with_invocation(args, invocation)
            .await
            .map(CapabilityExecutionResult::new)
    }

    /// Get capability metadata
    ///
    /// Provides schema, description, and other metadata
    /// used for validation and introspection
    fn metadata(&self) -> &RuntimeCapability;

    /// If this capability executes by spawning an external process, return
    /// an [`ExecRequest`] describing that process. The capability system will
    /// route it through the registered [`SandboxBackend`] instead of calling
    /// [`execute()`] directly.
    ///
    /// Return `None` (the default) for capabilities that don't spawn processes.
    fn to_exec_request(&self, args: &HashMap<String, Value>) -> Option<ExecRequest> {
        let _ = args; // suppress unused warning
        None
    }

}

/// Convert a sandbox [`ExecResult`] into a [`Value`] suitable for
/// returning from a capability invocation.
pub fn exec_result_to_value(result: ExecResult) -> Value {
    if result.timed_out {
        Value::String(format!(
            "[TIMEOUT after {:?}]\nstdout:\n{}\nstderr:\n{}",
            result.duration, result.stdout, result.stderr
        ))
    } else {
        // Match BashCapability's command-result shape.
        let mut payload = result.stdout.clone();
        if !result.stderr.is_empty() {
            if !payload.is_empty() {
                payload.push('\n');
            }
            payload.push_str("[stderr]\n");
            payload.push_str(&result.stderr);
        }
        if let Some(code) = result.exit_code
            && code != 0
        {
            let _ = write!(payload, "\n[exit code: {code}]");
        }
        Value::String(payload)
    }
}

/// Built-in echo capability for testing
pub struct EchoCapability {
    metadata: RuntimeCapability,
}

impl EchoCapability {
    pub fn new() -> Self {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "message": {
                    "type": "string",
                    "description": "Message to echo back"
                }
            },
            "required": ["message"]
        });

        Self {
            // Echo is side-effect-free: mark it read-only so it is not gated by
            // the invoke-site write boundary (and runs without a write lock).
            metadata: RuntimeCapability::new("echo", "Echo a message back to the caller", schema)
                .with_returns("string")
                .with_latency(10)
                .with_read_only(),
        }
    }
}

impl Default for EchoCapability {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl CapabilityExecutor for EchoCapability {
    async fn execute(&self, args: HashMap<String, Value>) -> CapabilityResult<Value> {
        let message = args
            .get("message")
            .and_then(|v| v.as_string())
            .ok_or_else(|| RuntimeError::Capability {
                capability: "echo".to_string(),
                message: "Missing or invalid 'message' argument".to_string(),
            })?;

        Ok(Value::String(format!("Echo: {}", message)))
    }

    fn metadata(&self) -> &RuntimeCapability {
        &self.metadata
    }
}

/// Mock search capability for testing and demos
///
/// Returns simulated search results for any query.
/// Useful for demonstrating tool invocation without external dependencies.
pub struct MockSearchCapability {
    metadata: RuntimeCapability,
}

impl MockSearchCapability {
    pub fn new() -> Self {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query"
                }
            },
            "required": ["query"]
        });

        Self {
            metadata: RuntimeCapability::new(
                "search",
                "Mock search capability that returns simulated results",
                schema,
            )
            .with_returns("string")
            .with_latency(50),
        }
    }
}

impl Default for MockSearchCapability {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl CapabilityExecutor for MockSearchCapability {
    async fn execute(&self, args: HashMap<String, Value>) -> CapabilityResult<Value> {
        let query = args
            .get("query")
            .and_then(|v| v.as_string())
            .map_or("unknown", |s| s.as_str());

        // Return mock search results
        Ok(Value::String(format!(
            "[Search Results for '{}']: 1. Overview of {}. 2. Key concepts in {}. 3. Related topics and applications.",
            query, query, query
        )))
    }

    fn metadata(&self) -> &RuntimeCapability {
        &self.metadata
    }
}
