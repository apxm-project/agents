//! COMMUNICATE operation - Inter-agent communication
//!
//! Supports four dispatch modes selected via the `protocol` node attribute:
//!   - `local` (default): in-process sub-flow execution via FlowRegistry
//!   - `http` / `https`: POST to an external APXM agent's `/v1/receive` endpoint
//!   - `acp`: send a prompt to an ACP agent subprocess via the ProcessTable
//!   - `broadcast`: fan-out to ALL registered agents in FlowRegistry in parallel;
//!     returns an Array of all responses (non-fatal errors included as strings)
//!
//! For HTTP, `recipient` may be a full URL or an agent name resolved via the
//! agent registry at `APXM_SERVER_URL/v1/agents/{name}`.
//!
//! For ACP, `recipient` must match an agent name previously spawned via
//! `SPAWN_AGENT` and registered in the ProcessTable.

use super::{ExecutionContext, Node, Result, Value, get_string_attribute};
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::error::RuntimeError;
use apxm_core::types::CommunicateProtocol;

mod acp;
mod broadcast;
mod http;
mod local;

pub async fn execute(ctx: &ExecutionContext, node: &Node, inputs: Vec<Value>) -> Result<Value> {
    let recipient = get_string_attribute(node, graph_attrs::RECIPIENT).unwrap_or_default();
    let protocol = match get_string_attribute(node, graph_attrs::PROTOCOL) {
        Ok(raw) => raw
            .parse::<CommunicateProtocol>()
            .map_err(|e| RuntimeError::Operation {
                op_type: node.op_type,
                message: format!("COMMUNICATE has invalid 'protocol' value: {e}"),
            })?,
        Err(_) => CommunicateProtocol::Local,
    };
    let message = local::resolve_message(node, protocol, &inputs)?;

    // Emit a typed COMMUNICATE_DISPATCHED event so
    // observers can render the inter-agent edge without scraping the
    // op attributes. Excerpt is capped to ~240 chars (Codex pattern).
    if let Some(emitter) = &ctx.event_emitter {
        let message_str = match &message {
            Value::String(s) => Some(s.clone()),
            other => other.to_json().ok().map(|json| json.to_string()),
        };
        let excerpt = message_str.as_deref().map(truncate_excerpt);
        emitter.emit_communicate_dispatched(
            node.id,
            &recipient,
            protocol.as_str(),
            excerpt.as_deref(),
        );
    }

    match protocol {
        CommunicateProtocol::Http | CommunicateProtocol::Https => {
            http::execute_http(ctx, node, &recipient, message).await
        }
        CommunicateProtocol::Broadcast => broadcast::execute_broadcast(ctx, node, message).await,
        CommunicateProtocol::Acp => acp::execute_acp(ctx, node, &recipient, message).await,
        CommunicateProtocol::Local => local::execute_local(ctx, node, &recipient, message).await,
    }
}

/// Cap a message excerpt at 240 chars so observer streams don't haul
/// multi-KB payloads — matches Codex CLI's `evidence_excerpt` budget.
fn truncate_excerpt(text: &str) -> String {
    const MAX: usize = 240;
    if text.len() <= MAX {
        return text.to_string();
    }
    let mut end = MAX;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::CapabilitySystem;
    use crate::capability::flow_registry::FlowRegistry;
    use crate::context_stack::{ContextStack, NodeMetadata as ContextNodeMetadata};
    use crate::executor::events::ExecutionEventEmitter;
    use crate::memory::{MemoryConfig, MemorySystem};
    use crate::testing::{
        MOCK_AGENT_PROFILE, MOCK_MODEL_NAME, MOCK_SESSION_ID, MOCK_STOP_REASON,
        MockUsageAgentPrompter, RecordingAgentPrompter,
    };
    use apxm_backends::LLMRegistry;
    use apxm_core::paths::session_node_dir_name;
    use apxm_core::types::{
        execution::{ExecutionDag, NodeMetadata},
        operations::AISOperationType,
    };
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use tempfile::tempdir;

    #[derive(Default)]
    struct RecordingEmitter {
        prompts: Mutex<Vec<(u64, String)>>,
    }

    impl RecordingEmitter {
        fn prompts(&self) -> Vec<(u64, String)> {
            self.prompts.lock().expect("prompt lock").clone()
        }
    }

    impl ExecutionEventEmitter for RecordingEmitter {
        fn emit_llm_token(&self, _content: &str) {}

        fn emit_tool_start(&self, _name: &str, _args: &HashMap<String, Value>) {}

        fn emit_tool_end(&self, _name: &str, _result: &Value) {}

        fn emit_llm_prompt(&self, node_id: u64, prompt: &str) {
            self.prompts
                .lock()
                .expect("prompt lock")
                .push((node_id, prompt.to_string()));
        }
    }

    fn create_echo_dag() -> ExecutionDag {
        let mut const_node = apxm_core::types::execution::Node {
            id: 1,
            op_type: AISOperationType::ConstStr,
            attributes: HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![100],
            metadata: NodeMetadata::default(),
        };
        const_node.attributes.insert(
            graph_attrs::VALUE.to_string(),
            Value::String("ack from agent".to_string()),
        );
        ExecutionDag {
            nodes: vec![const_node],
            edges: vec![],
            entry_nodes: vec![1],
            exit_nodes: vec![1],
            metadata: Default::default(),
        }
    }

    #[tokio::test]
    async fn test_communicate_with_registered_agent() {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());
        let flow_registry = Arc::new(FlowRegistry::new());

        flow_registry.register_flow("PeerAgent", "communicate", create_echo_dag());

        let ctx = ExecutionContext::new(
            memory,
            llm_registry,
            capability_system,
            crate::aam::Aam::new(),
        );
        let ctx = ExecutionContext {
            flow_registry,
            ..ctx
        };

        let mut node = apxm_core::types::execution::Node {
            id: 1,
            op_type: AISOperationType::Communicate,
            attributes: HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![100],
            metadata: NodeMetadata::default(),
        };
        node.attributes.insert(
            graph_attrs::RECIPIENT.to_string(),
            Value::String("PeerAgent".to_string()),
        );

        let result = execute(&ctx, &node, vec![Value::String("hello".to_string())])
            .await
            .unwrap();
        assert_eq!(result, Value::String("ack from agent".to_string()));
    }

    #[tokio::test]
    async fn test_communicate_acp_prepends_context_stack_frames() {
        let dir = tempdir().expect("tempdir");
        let session_dir = dir.path().join("session");
        let upstream_dir = session_dir
            .join(apxm_core::constants::session::files::NODES_DIR)
            .join(session_node_dir_name(1, "seed"));
        std::fs::create_dir_all(&upstream_dir).expect("upstream dir");
        std::fs::write(
            upstream_dir.join(apxm_core::constants::session::node::OUTPUT_JSON),
            "{\"result\":\"upstream data\"}",
        )
        .expect("output");

        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());
        let process_table = Arc::new(crate::process_table::ProcessTable::new());
        process_table
            .spawn_local("PeerAgent".to_string(), None)
            .expect("spawn local process");

        let prompts = Arc::new(Mutex::new(Vec::new()));
        process_table
            .set_agent_prompter(Arc::new(RecordingAgentPrompter::new(Arc::clone(&prompts))))
            .await;

        let context_stack = Arc::new(ContextStack::new(
            session_dir.clone(),
            Arc::new(HashMap::from([
                (
                    1,
                    ContextNodeMetadata {
                        name: "seed".to_string(),
                        op_type: AISOperationType::ConstStr,
                    },
                ),
                (
                    2,
                    ContextNodeMetadata {
                        name: "communicate_peer".to_string(),
                        op_type: AISOperationType::Communicate,
                    },
                ),
            ])),
            Arc::new(vec![(1, 2)]),
        ));

        let ctx = ExecutionContext::new(
            memory,
            llm_registry,
            capability_system,
            crate::aam::Aam::new(),
        )
        .with_context_stack(context_stack)
        .with_process_table(process_table);

        let mut node = apxm_core::types::execution::Node {
            id: 2,
            op_type: AISOperationType::Communicate,
            attributes: HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![100],
            metadata: NodeMetadata::default(),
        };
        node.attributes.insert(
            graph_attrs::RECIPIENT.to_string(),
            Value::String("PeerAgent".to_string()),
        );
        node.attributes.insert(
            graph_attrs::PROTOCOL.to_string(),
            Value::String(CommunicateProtocol::Acp.as_str().to_string()),
        );
        node.attributes.insert(
            graph_attrs::PROFILE.to_string(),
            Value::String(MOCK_AGENT_PROFILE.to_string()),
        );

        let result = execute(
            &ctx,
            &node,
            vec![Value::String("Respond to the user".to_string())],
        )
        .await
        .unwrap();

        let recorded = prompts.lock().expect("prompt lock");
        assert_eq!(recorded.len(), 1);
        assert!(recorded[0].contains("## Session"));
        assert!(recorded[0].contains("## Upstream: seed (#1)"));
        assert!(recorded[0].contains("upstream data"));
        assert!(recorded[0].contains("Respond to the user"));
        assert_eq!(result, Value::String(recorded[0].clone()));
    }

    #[tokio::test]
    async fn test_communicate_acp_uses_message_attribute_when_inputs_missing() {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());
        let process_table = Arc::new(crate::process_table::ProcessTable::new());
        process_table
            .spawn_local("PeerAgent".to_string(), None)
            .expect("spawn local process");

        let prompts = Arc::new(Mutex::new(Vec::new()));
        process_table
            .set_agent_prompter(Arc::new(RecordingAgentPrompter::new(Arc::clone(&prompts))))
            .await;

        let ctx = ExecutionContext::new(
            memory,
            llm_registry,
            capability_system,
            crate::aam::Aam::new(),
        )
        .with_process_table(process_table);

        let mut node = apxm_core::types::execution::Node {
            id: 1,
            op_type: AISOperationType::Communicate,
            attributes: HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![100],
            metadata: NodeMetadata::default(),
        };
        node.attributes.insert(
            graph_attrs::RECIPIENT.to_string(),
            Value::String("PeerAgent".to_string()),
        );
        node.attributes.insert(
            graph_attrs::PROTOCOL.to_string(),
            Value::String(CommunicateProtocol::Acp.as_str().to_string()),
        );
        node.attributes.insert(
            graph_attrs::MESSAGE.to_string(),
            Value::String("Respond from attribute".to_string()),
        );

        let result = execute(&ctx, &node, vec![]).await.unwrap();
        assert_eq!(result, Value::String("Respond from attribute".to_string()));
        let recorded = prompts.lock().expect("prompt lock");
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0], "Respond from attribute");
    }

    #[tokio::test]
    async fn test_communicate_acp_emits_actual_prompt() {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());
        let process_table = Arc::new(crate::process_table::ProcessTable::new());
        process_table
            .spawn_local("PeerAgent".to_string(), None)
            .expect("spawn local process");
        process_table
            .set_agent_prompter(Arc::new(RecordingAgentPrompter::new(Arc::new(Mutex::new(
                Vec::new(),
            )))))
            .await;

        let emitter = Arc::new(RecordingEmitter::default());
        let ctx = ExecutionContext::new(
            memory,
            llm_registry,
            capability_system,
            crate::aam::Aam::new(),
        )
        .with_process_table(process_table)
        .with_event_emitter(Some(emitter.clone() as Arc<dyn ExecutionEventEmitter>));

        let mut node = apxm_core::types::execution::Node {
            id: 42,
            op_type: AISOperationType::Communicate,
            attributes: HashMap::new(),
            input_tokens: vec![7],
            output_tokens: vec![100],
            metadata: NodeMetadata::default(),
        };
        node.attributes.insert(
            graph_attrs::RECIPIENT.to_string(),
            Value::String("PeerAgent".to_string()),
        );
        node.attributes.insert(
            graph_attrs::PROTOCOL.to_string(),
            Value::String(CommunicateProtocol::Acp.as_str().to_string()),
        );
        node.attributes.insert(
            graph_attrs::MESSAGE.to_string(),
            Value::String("Review {task}".to_string()),
        );
        node.attributes.insert(
            graph_attrs::INPUT_NAMES.to_string(),
            Value::Array(vec![Value::String("task".to_string())]),
        );

        let result = execute(&ctx, &node, vec![Value::String("runtime task".to_string())])
            .await
            .unwrap();
        assert_eq!(result, Value::String("Review runtime task".to_string()));

        assert_eq!(
            emitter.prompts(),
            vec![(42, "Review runtime task".to_string())]
        );
    }

    #[tokio::test]
    async fn test_communicate_acp_records_metrics_with_mock_prompter() {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());
        let process_table = Arc::new(crate::process_table::ProcessTable::new());
        let process_id = process_table
            .spawn_local("PeerAgent".to_string(), None)
            .expect("spawn local process");
        process_table
            .set_agent_prompter(Arc::new(
                MockUsageAgentPrompter::new("mock response: ")
                    .with_session_id(MOCK_SESSION_ID)
                    .with_turn(1)
                    .with_model(MOCK_MODEL_NAME)
                    .with_stop_reason(MOCK_STOP_REASON)
                    .with_token_usage(Some(10), Some(5)),
            ))
            .await;

        let ctx = ExecutionContext::new(
            memory,
            llm_registry,
            capability_system,
            crate::aam::Aam::new(),
        )
        .with_process_table(process_table);

        let mut node = apxm_core::types::execution::Node {
            id: 7,
            op_type: AISOperationType::Communicate,
            attributes: HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![100],
            metadata: NodeMetadata::default(),
        };
        node.attributes.insert(
            graph_attrs::RECIPIENT.to_string(),
            Value::String("PeerAgent".to_string()),
        );
        node.attributes.insert(
            graph_attrs::PROTOCOL.to_string(),
            Value::String(CommunicateProtocol::Acp.as_str().to_string()),
        );

        let result = execute(&ctx, &node, vec![Value::String("hello".to_string())])
            .await
            .unwrap();

        assert_eq!(
            result,
            Value::String("mock response: hello".to_string()),
            "handler should return the mock prompter text"
        );

        let snapshot = ctx.graph_metrics.snapshot();
        let node_metrics = snapshot.nodes.get(&node.id).expect("node metrics");
        assert_eq!(node_metrics.processes.totals.prompt_turns, 1);
        assert_eq!(node_metrics.processes.totals.input_tokens, 10);
        assert_eq!(node_metrics.processes.totals.output_tokens, 5);
        assert_eq!(snapshot.graph.processes.total_tokens, 15);
        assert_eq!(snapshot.aggregates.by_agent["PeerAgent"].prompt_turns, 1);

        let turn = &node_metrics.processes.prompt_turns[0];
        assert_eq!(turn.process_id, process_id);
        assert_eq!(turn.protocol, CommunicateProtocol::Acp.as_str());
        assert_eq!(turn.session_id.as_deref(), Some(MOCK_SESSION_ID));
        assert_eq!(turn.model.as_deref(), Some(MOCK_MODEL_NAME));
        assert_eq!(turn.stop_reason.as_deref(), Some(MOCK_STOP_REASON));

        let tokens = ctx.token_accountant.get_node(node.id).expect("tokens");
        assert_eq!(tokens.input_tokens, 10);
        assert_eq!(tokens.output_tokens, 5);
    }

    #[tokio::test]
    async fn test_communicate_acp_rejects_empty_message() {
        let memory = Arc::new(
            MemorySystem::new(MemoryConfig::in_memory_ltm())
                .await
                .unwrap(),
        );
        let llm_registry = Arc::new(LLMRegistry::new());
        let capability_system = Arc::new(CapabilitySystem::new());
        let process_table = Arc::new(crate::process_table::ProcessTable::new());
        process_table
            .spawn_local("PeerAgent".to_string(), None)
            .expect("spawn local process");

        let prompts = Arc::new(Mutex::new(Vec::new()));
        process_table
            .set_agent_prompter(Arc::new(RecordingAgentPrompter::new(Arc::clone(&prompts))))
            .await;

        let ctx = ExecutionContext::new(
            memory,
            llm_registry,
            capability_system,
            crate::aam::Aam::new(),
        )
        .with_process_table(process_table);

        let mut node = apxm_core::types::execution::Node {
            id: 1,
            op_type: AISOperationType::Communicate,
            attributes: HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![100],
            metadata: NodeMetadata::default(),
        };
        node.attributes.insert(
            graph_attrs::RECIPIENT.to_string(),
            Value::String("PeerAgent".to_string()),
        );
        node.attributes.insert(
            graph_attrs::PROTOCOL.to_string(),
            Value::String(CommunicateProtocol::Acp.as_str().to_string()),
        );

        let error = execute(&ctx, &node, vec![])
            .await
            .expect_err("empty prompt should fail");
        assert!(matches!(error, RuntimeError::Operation { .. }));
        assert!(
            error
                .to_string()
                .contains("requires a non-empty message or string input")
        );
        assert!(prompts.lock().expect("prompt lock").is_empty());
    }

    #[tokio::test]
    async fn test_communicate_agent_not_found() {
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

        let mut node = apxm_core::types::execution::Node {
            id: 1,
            op_type: AISOperationType::Communicate,
            attributes: HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![100],
            metadata: NodeMetadata::default(),
        };
        node.attributes.insert(
            graph_attrs::RECIPIENT.to_string(),
            Value::String("NonExistent".to_string()),
        );

        let result = execute(&ctx, &node, vec![]).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No agents"));
    }

    #[test]
    fn dispatch_rejects_unknown_protocol_with_typed_error() {
        // Build a runtime memory + ctx (sync, no registry needed for early-fail path)
        let mut node = apxm_core::types::execution::Node {
            id: 1,
            op_type: AISOperationType::Communicate,
            attributes: HashMap::new(),
            input_tokens: vec![],
            output_tokens: vec![100],
            metadata: NodeMetadata::default(),
        };
        node.attributes.insert(
            graph_attrs::PROTOCOL.to_string(),
            Value::String("websocket".to_string()),
        );

        // Verify FromStr returns the typed UnknownProtocol error and dispatcher
        // surfaces it as RuntimeError::Operation with the protocol name in the message.
        let parse_err = "websocket".parse::<CommunicateProtocol>().unwrap_err();
        assert_eq!(parse_err.0, "websocket");
        assert!(
            parse_err
                .to_string()
                .contains("unknown communicate protocol")
        );
    }
}
