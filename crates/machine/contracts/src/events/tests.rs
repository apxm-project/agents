//! Unit tests for the core event envelope.

use std::collections::HashMap;
use std::sync::Arc;

use super::event::{ApxmEvent, EventSource};
use super::kind::{self, CORE_EVENT_KINDS};
use super::payload::*;

/// A valid `EventMeta` JSON object, used to build well-formed envelope
/// fixtures for the negative-path test below without depending on
/// `ApxmEvent::root`'s serialization.
fn sample_meta_json() -> serde_json::Value {
    serde_json::json!({
        "seq": 0,
        "timestamp": "2026-07-09T00:00:00Z",
        "trace_id": "trace-1",
        "source": "runtime",
        "span_id": "span-1",
        "parent_span_id": null,
    })
}

/// Positive: for each real edge kind, a `GraphEdgePayload` round-trips its
/// `edge_kind` field through serialize -> JSON -> deserialize without the
/// envelope's own `"kind": "graph_edge"` discriminator clobbering it.
#[test]
fn graph_edge_round_trips_edge_kind_through_serialize_deserialize() {
    for edge_kind in ["dispatch", "tool_invocation", "synthesis_feed"] {
        let event = ApxmEvent::root(
            GraphEdgePayload {
                from_node_id: 1,
                to_node_id: 2,
                edge_kind: edge_kind.to_string(),
            },
            EventSource::Runtime,
            "trace-1",
        );

        let value = serde_json::to_value(&event).expect("serialize ApxmEvent");
        let payload_json = value.get("payload").expect("payload key present");

        // Envelope discriminator is intact...
        assert_eq!(
            payload_json.get("kind").and_then(serde_json::Value::as_str),
            Some("graph_edge"),
            "envelope kind must remain graph_edge for edge_kind={edge_kind}"
        );
        // ...and the payload's own discriminator survived alongside it.
        assert_eq!(
            payload_json
                .get("edge_kind")
                .and_then(serde_json::Value::as_str),
            Some(edge_kind),
            "edge_kind must survive serialization unclobbered"
        );

        let round_tripped: ApxmEvent =
            serde_json::from_value(value).expect("deserialize ApxmEvent");
        let decoded = round_tripped
            .payload
            .downcast_ref::<GraphEdgePayload>()
            .expect("payload decodes back to GraphEdgePayload");
        assert_eq!(decoded.from_node_id, 1);
        assert_eq!(decoded.to_node_id, 2);
        assert_eq!(decoded.edge_kind, edge_kind);
    }
}

/// Negative: a pre-fix corrupted wire shape (no `edge_kind`, only the
/// overwritten envelope `"kind":"graph_edge"`) must fail closed with a
/// typed error naming the missing field — never silently default.
#[test]
fn graph_edge_missing_edge_kind_fails_closed_not_default() {
    let value = serde_json::json!({
        "meta": sample_meta_json(),
        "payload": {
            "kind": "graph_edge",
            "from_node_id": 1,
            "to_node_id": 2,
        },
    });

    let result = serde_json::from_value::<ApxmEvent>(value);
    let err = result.expect_err("missing edge_kind must fail closed, not default");
    let message = err.to_string();
    assert!(
        message.contains("edge_kind"),
        "error must name the missing edge_kind field, got: {message}"
    );
}

/// Regression pin: no `GraphEdgePayload` value, however constructed, can
/// produce a serialized envelope whose `"kind"` differs from the fixed
/// literal `"graph_edge"` — the payload's own discriminator lives in a
/// structurally separate field (`edge_kind`) that can never collide with
/// the envelope's reserved `"kind"` key.
#[test]
fn graph_edge_envelope_kind_is_never_shadowed_by_payload_field() {
    for edge_kind in [
        "dispatch",
        "tool_invocation",
        "synthesis_feed",
        "",
        "graph_edge",
        "kind",
        "some_other_event_kind_name",
    ] {
        let event = ApxmEvent::root(
            GraphEdgePayload {
                from_node_id: 7,
                to_node_id: 9,
                edge_kind: edge_kind.to_string(),
            },
            EventSource::Runtime,
            "trace-2",
        );

        let value = serde_json::to_value(&event).expect("serialize ApxmEvent");
        let envelope_kind = value
            .get("payload")
            .and_then(|payload| payload.get("kind"))
            .and_then(serde_json::Value::as_str);
        assert_eq!(
            envelope_kind,
            Some("graph_edge"),
            "envelope kind must never be shadowed by an arbitrary edge_kind value {edge_kind:?}"
        );
    }
}

/// One concrete, representative payload value per `CORE_EVENT_KINDS` entry.
/// Every field is populated with a distinguishable, non-default value so a
/// round-trip that dropped or renamed a field is guaranteed to be caught.
fn representative_core_payloads() -> Vec<Box<dyn EventPayload>> {
    vec![
        Box::new(TokenPayload {
            text: "hello".to_string(),
        }),
        Box::new(ThoughtPayload {
            text: "thinking it through".to_string(),
            summary: Some("short summary".to_string()),
        }),
        Box::new(ToolCallPayload {
            id: "call-1".to_string(),
            name: "web_search".to_string(),
            arguments: serde_json::json!({"q": "apxm"}),
        }),
        Box::new(LlmDonePayload {
            content: "final answer".to_string(),
            model: "gpt-test".to_string(),
            finish_reason: FinishReasonPayload {
                reason: "stop".to_string(),
            },
            usage: UsagePayload {
                input_tokens: 10,
                output_tokens: 20,
            },
            tool_calls: vec![ToolCallPayload {
                id: "call-2".to_string(),
                name: "read_file".to_string(),
                arguments: serde_json::json!({}),
            }],
            response_id: Some("resp-1".to_string()),
        }),
        Box::new(LlmStepCompletedPayload {
            node_id: 1,
            step_number: 2,
            model: "gpt-test".to_string(),
            finish_reason: FinishReasonPayload {
                reason: "tool_use".to_string(),
            },
            usage: LlmStepUsagePayload {
                input_tokens: 11,
                output_tokens: 7,
                cached_input_tokens: 3,
                reasoning_output_tokens: 2,
            },
            performance: LlmStepPerformancePayload {
                latency_ms: 12.5,
                prefill_ms: 4.0,
                decode_ms: 8.5,
            },
            tool_call_count: 1,
        }),
        Box::new(LlmPromptPayload {
            node_id: 1,
            node_name: Some("ask-node".to_string()),
            prompt: RedactedContent::from_text("a secret prompt"),
        }),
        Box::new(UsagePayload {
            input_tokens: 1,
            output_tokens: 2,
        }),
        Box::new(RetryPayload {
            attempt: 2,
            reason: "rate_limited".to_string(),
            retry_after_secs: 1.5,
            backend: Some("openai".to_string()),
        }),
        Box::new(WarningPayload {
            code: "W1".to_string(),
            message: "careful now".to_string(),
        }),
        Box::new(CitationPayload {
            citations: vec![Citation {
                url: Some("https://example.com".to_string()),
                title: Some("Example".to_string()),
                start_index: Some(0),
                end_index: Some(5),
            }],
        }),
        Box::new(ProviderEventPayload {
            provider: "openai".to_string(),
            event_type: "response.delta".to_string(),
            data: serde_json::json!({"a": 1}),
        }),
        Box::new(OperationStartPayload {
            node_id: 1,
            op_type: crate::types::operations::AISOperationType::Ask,
            context: Some(serde_json::json!({"x": 1})),
        }),
        Box::new(OperationEndPayload {
            node_id: 1,
            op_type: crate::types::operations::AISOperationType::Ask,
            duration_ms: 100,
            success: true,
        }),
        Box::new(NodeOutputPayload {
            node_id: 1,
            node_name: Some("ask-node".to_string()),
            output: RedactedContent::from_json(&serde_json::json!({"y": 2})),
        }),
        Box::new(NodeMetricsPayload {
            node_id: 1,
            node_name: Some("ask-node".to_string()),
            metrics: crate::types::NodeMetrics::new(1),
        }),
        Box::new(ToolStartPayload {
            name: "web_search".to_string(),
            args: HashMap::from([("q".to_string(), serde_json::json!("apxm"))]),
        }),
        Box::new(ToolEndPayload {
            name: "web_search".to_string(),
            result: serde_json::json!({"ok": true}),
        }),
        Box::new(PlanCreatedPayload {
            plan_id: "plan-1".to_string(),
            steps: 3,
        }),
        Box::new(PlanStepStartedPayload {
            plan_id: "plan-1".to_string(),
            step_index: 0,
        }),
        Box::new(PlanStepCompletedPayload {
            plan_id: "plan-1".to_string(),
            step_index: 0,
            success: true,
        }),
        Box::new(PlanWorkflowEmittedPayload {
            plan_id: "plan-1".to_string(),
            generating_model: "gpt-test".to_string(),
            node_count: 3,
            task_ids: vec![1, 2, 3],
            parallel_fanout_max: 2,
        }),
        Box::new(WorkflowStartedPayload {
            workflow_name: "wf".to_string(),
            session_dir: "/tmp/wf".to_string(),
            step_count: 2,
        }),
        Box::new(WorkflowStepStartedPayload {
            workflow_name: "wf".to_string(),
            workflow_session_dir: "/tmp/wf".to_string(),
            step_id: "s1".to_string(),
            step_index: 0,
            step_count: 2,
        }),
        Box::new(WorkflowStepCompletedPayload {
            workflow_name: "wf".to_string(),
            workflow_session_dir: "/tmp/wf".to_string(),
            step_id: "s1".to_string(),
            step_index: 0,
            status: "success".to_string(),
            success: true,
            duration_ms: 10,
            session_dir: Some("/tmp/wf/s1".to_string()),
            error: None,
        }),
        Box::new(WorkflowFinishedPayload {
            workflow_name: "wf".to_string(),
            session_dir: "/tmp/wf".to_string(),
            status: "success".to_string(),
            success: true,
            duration_ms: 20,
            step_count: 2,
        }),
        Box::new(ExecutionStartedPayload {
            execution_id: "exec-1".to_string(),
            args: vec!["a".to_string()],
            user_text: Some("hi".to_string()),
        }),
        Box::new(ExecuteCompletePayload {
            result: serde_json::json!({"done": true}),
        }),
        Box::new(MemoryReadPayload {
            scope: "short_term".to_string(),
            key: "k".to_string(),
        }),
        Box::new(MemoryWritePayload {
            scope: "short_term".to_string(),
            key: "k".to_string(),
        }),
        Box::new(CheckpointSavedPayload {
            checkpoint_id: "cp1".to_string(),
        }),
        Box::new(CheckpointRestoredPayload {
            checkpoint_id: "cp1".to_string(),
        }),
        Box::new(SchedulerDecisionPayload {
            node_id: 1,
            delay_ms: 5,
            reason: "backoff".to_string(),
        }),
        Box::new(ModelRouteDecisionPayload {
            backend: "vllm".to_string(),
            model: Some("llama-70b".to_string()),
            was_failover: false,
            reason: "cost".to_string(),
            rejected_candidates: vec![ModelRouteRejectionPayload {
                candidate: "gpt-slow".to_string(),
                backend: "openai".to_string(),
                reason_kind: "circuit_breaker_open".to_string(),
                reason: "backend circuit breaker is open".to_string(),
            }],
        }),
        Box::new(AgentRouteDecisionPayload {
            id: "req-1".to_string(),
            profile: Some("planner-profile".to_string()),
            source: "selected".to_string(),
            reason: "capability fit".to_string(),
            required_capabilities: vec!["read".to_string()],
            rejected_candidates: vec![AgentRouteRejectionPayload {
                profile: "reviewer-profile".to_string(),
                missing_capabilities: vec!["execute".to_string()],
                reason: "missing required capability".to_string(),
            }],
        }),
        Box::new(HeadOfLineBlockPayload {
            blocker_node: 1,
            blocked_node: 2,
            wait_ms: 50,
            reason: "busy".to_string(),
        }),
        Box::new(GpuUtilizationPayload {
            gpu_id: 0,
            utilization_pct: 50.0,
            memory_pct: 30.0,
        }),
        Box::new(TokenUsagePayload {
            node_id: 1,
            input_tokens: 10,
            output_tokens: 20,
        }),
        Box::new(MemoizationHitPayload { node_id: 1 }),
        Box::new(ErrorPayload {
            message: "boom".to_string(),
            status: Some("500".to_string()),
            recoverable: false,
        }),
        Box::new(AgentSpawnedPayload {
            node_id: 1,
            agent_code: "agent-1".to_string(),
            parent_execution_id: "exec-1".to_string(),
            profile: Some("acp".to_string()),
            process_id: Some("pid-1".to_string()),
            scope_policy: Some("scoped".to_string()),
        }),
        Box::new(CommunicateDispatchedPayload {
            node_id: 1,
            target_agent: "agent-2".to_string(),
            protocol: "local".to_string(),
            message_excerpt: Some("hi".to_string()),
        }),
        Box::new(GraphEdgePayload {
            from_node_id: 1,
            to_node_id: 2,
            edge_kind: "dispatch".to_string(),
        }),
        Box::new(ContextCompactedPayload {
            original_tokens: 100,
            new_tokens: 50,
        }),
        Box::new(ModelReroutedPayload {
            original_model: "gpt-test".to_string(),
            new_model: "llama-70b".to_string(),
            reason: "failover".to_string(),
        }),
        Box::new(CancelledPayload {
            reason: Some("user_requested".to_string()),
        }),
        Box::new(LoopDetectedPayload {
            pattern: "A->B->A".to_string(),
            iterations: 3,
        }),
        Box::new(ContextWindowWarningPayload {
            current_tokens: 900,
            max_tokens: 1000,
            utilization_pct: 90.0,
        }),
        Box::new(SessionStartPayload {
            session_id: "sess-1".to_string(),
        }),
        Box::new(SessionEndPayload {
            session_id: "sess-1".to_string(),
            total_turns: 5,
        }),
        Box::new(TurnBoundaryPayload {
            turn_number: 1,
            direction: TurnDirection::Request,
        }),
        Box::new(TurnStartedPayload {
            execution_id: "exec-1".to_string(),
            turn_id: Some("turn-1".to_string()),
            coordinator_label: Some("Cleo".to_string()),
        }),
        Box::new(TurnCompletePayload {
            execution_id: "exec-1".to_string(),
            duration_ms: 100,
            had_answer: true,
        }),
        Box::new(TurnAbortedPayload {
            execution_id: "exec-1".to_string(),
            duration_ms: 100,
            reason: "error".to_string(),
            error_message_safe: Some("boom".to_string()),
        }),
        Box::new(SubagentSpawnBeginPayload {
            agent_code: "agent-1".to_string(),
            agent_name: Some("Agent One".to_string()),
            agent_type: Some("module_agent".to_string()),
            module_key: Some("mod-1".to_string()),
            autonomy_policy: Some("supervised".to_string()),
            parent_span_id: Some("span-0".to_string()),
        }),
        Box::new(SubagentSpawnEndPayload {
            agent_code: "agent-1".to_string(),
        }),
        Box::new(SubagentLlmCallBeginPayload {
            agent_code: "agent-1".to_string(),
            model: "gpt-test".to_string(),
            backend: "openai".to_string(),
            tool_manifest_count: 2,
        }),
        Box::new(SubagentLlmCallEndPayload {
            agent_code: "agent-1".to_string(),
            finish_reason: "stop".to_string(),
            usage: UsagePayload {
                input_tokens: 5,
                output_tokens: 10,
            },
            content_len: 42,
        }),
        Box::new(ToolCallBeginPayload {
            agent_code: "agent-1".to_string(),
            tool_name: "web_search".to_string(),
            argument_keys: vec!["q".to_string()],
        }),
        Box::new(ToolCallEndPayload {
            agent_code: "agent-1".to_string(),
            tool_name: "web_search".to_string(),
            result_keys: vec!["r".to_string()],
            status: "ok".to_string(),
            latency_ms: 12,
        }),
        Box::new(SubagentDonePayload {
            agent_code: "agent-1".to_string(),
            total_tool_calls: 3,
            usage_total: UsagePayload {
                input_tokens: 100,
                output_tokens: 200,
            },
            evidence_excerpt: Some("evidence".to_string()),
        }),
        Box::new(SubagentFailedPayload {
            agent_code: "agent-1".to_string(),
            error_class: "timeout".to_string(),
            error_message_safe: "timed out".to_string(),
        }),
        Box::new(AgentMessagePayload {
            text: "final answer".to_string(),
            item_id: Some("item-1".to_string()),
            response_id: Some("resp-1".to_string()),
            usage: Some(UsagePayload {
                input_tokens: 1,
                output_tokens: 2,
            }),
        }),
        Box::new(ApprovalRequestPayload {
            agent_code: "agent-1".to_string(),
            tool_name: "shell".to_string(),
            approval_id: "appr-1".to_string(),
            risk_level: "high".to_string(),
        }),
        Box::new(ApprovalResolvedPayload {
            approval_id: "appr-1".to_string(),
            decision: "approved".to_string(),
        }),
    ]
}

/// Positive: every `CORE_EVENT_KINDS` entry has exactly one representative
/// fixture above, and each one serializes into an `ApxmEvent`, round-trips
/// through JSON, and comes back with the same kind name and every field
/// intact (compared as JSON so field order can't hide a regression).
#[test]
fn all_core_event_kinds_round_trip() {
    let payloads = representative_core_payloads();

    let mut covered = std::collections::BTreeSet::new();
    for payload in payloads {
        let kind = payload.event_kind();
        covered.insert(kind.name());

        let pre_json = payload.to_json();
        let event = ApxmEvent::root_shared(Arc::from(payload), EventSource::Runtime, "trace-rt");

        let value = serde_json::to_value(&event)
            .unwrap_or_else(|err| panic!("serialize {} failed: {err}", kind.name()));
        let round_tripped: ApxmEvent = serde_json::from_value(value)
            .unwrap_or_else(|err| panic!("deserialize {} failed: {err}", kind.name()));

        assert_eq!(
            round_tripped.kind().name(),
            kind.name(),
            "kind must survive round-trip"
        );
        assert_eq!(
            round_tripped.payload.to_json(),
            pre_json,
            "every field of {} must survive round-trip unclobbered",
            kind.name()
        );
    }

    let expected: std::collections::BTreeSet<&'static str> =
        CORE_EVENT_KINDS.iter().map(|k| k.name()).collect();
    assert_eq!(
        covered, expected,
        "representative_core_payloads must cover every CORE_EVENT_KINDS entry, no more, no less"
    );
}

/// Terminal-set regression pin. Topology events resolved mid-run
/// (`agent_spawned`, `communicate_dispatched`, `graph_edge`) must never be
/// `terminal` — a consumer that trusts `is_terminal()` to close a live feed
/// would otherwise stop rendering mid-run on a topology event and hide
/// everything after, which looks like success and is strictly worse than
/// under-classifying. `cancelled` is a typed signal followed by the one
/// run-ending `execute_complete` event, so it must not close the feed early.
#[test]
fn terminal_kinds_exclude_topology_events() {
    assert!(!kind::AGENT_SPAWNED.is_terminal());
    assert!(!kind::COMMUNICATE_DISPATCHED.is_terminal());
    assert!(!kind::GRAPH_EDGE.is_terminal());
    assert!(!kind::LLM_DONE.is_terminal());
    assert!(!kind::LLM_STEP_COMPLETED.is_terminal());
    assert!(!kind::CANCELLED.is_terminal());

    assert!(kind::TURN_COMPLETE.is_terminal());
    assert!(kind::SESSION_END.is_terminal());
    assert!(kind::TURN_ABORTED.is_terminal());
    assert!(kind::ERROR.is_terminal());
}
