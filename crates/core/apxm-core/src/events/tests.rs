//! Unit tests for the core event envelope.

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use serde::{Deserialize, Serialize};

    use crate::events::event::{ApxmEvent, EventSource, SkillEventProvenance};
    use crate::events::kind;
    use crate::events::payload::*;
    use crate::events::{
        EventCategory, EventKind, EventPayloadRegistry, EventRegistryError, register_event_payload,
        registered_event_kind, registered_event_kinds,
    };
    use crate::types::operations::AISOperationType;
    use crate::types::{NodeMetrics, OperationMetric};

    const TEST_EXTENSION_KIND: EventKind =
        EventKind::new("test_extension_event", EventCategory::Lifecycle, false);
    const TEST_EXTENSION_MALFORMED_KIND: EventKind = EventKind::new(
        "test_extension_schema_error",
        EventCategory::Lifecycle,
        false,
    );
    const TEST_EXTENSION_DUPLICATE_KIND: EventKind =
        EventKind::new("test_extension_duplicate", EventCategory::Lifecycle, false);
    const TEST_LOCAL_EXTENSION_KIND: EventKind = EventKind::new(
        "test_local_extension_event",
        EventCategory::Lifecycle,
        false,
    );
    const TEST_LOCAL_EXTENSION_MALFORMED_KIND: EventKind = EventKind::new(
        "test_local_extension_schema_error",
        EventCategory::Lifecycle,
        false,
    );
    const TEST_PRECEDENCE_EXTENSION_KIND: EventKind =
        EventKind::new("test_extension_precedence", EventCategory::Lifecycle, false);

    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct TestExtensionPayload {
        id: String,
        count: u64,
    }
    crate::impl_event_payload!(TestExtensionPayload, TEST_EXTENSION_KIND);

    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct TestExtensionMalformedPayload {
        id: String,
        count: u64,
    }
    crate::impl_event_payload!(TestExtensionMalformedPayload, TEST_EXTENSION_MALFORMED_KIND);

    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct TestExtensionDuplicatePayload {
        id: String,
    }
    crate::impl_event_payload!(TestExtensionDuplicatePayload, TEST_EXTENSION_DUPLICATE_KIND);

    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct TestLocalExtensionPayload {
        id: String,
    }
    crate::impl_event_payload!(TestLocalExtensionPayload, TEST_LOCAL_EXTENSION_KIND);

    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct TestLocalExtensionMalformedPayload {
        count: u64,
    }
    crate::impl_event_payload!(
        TestLocalExtensionMalformedPayload,
        TEST_LOCAL_EXTENSION_MALFORMED_KIND
    );

    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct TestGlobalPrecedencePayload {
        global_id: String,
    }
    crate::impl_event_payload!(TestGlobalPrecedencePayload, TEST_PRECEDENCE_EXTENSION_KIND);

    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct TestLocalPrecedencePayload {
        local_id: String,
    }
    crate::impl_event_payload!(TestLocalPrecedencePayload, TEST_PRECEDENCE_EXTENSION_KIND);

    fn roundtrip<T>(payload: T)
    where
        T: EventPayload + Clone,
    {
        let event = ApxmEvent::root(payload, EventSource::Runtime, "test-trace-id");
        let json = serde_json::to_string(&event).expect("serialize");
        let back: ApxmEvent = serde_json::from_str(&json).expect("deserialize");
        let json2 = serde_json::to_string(&back).expect("re-serialize");
        assert_eq!(json, json2);
    }

    fn raw_event_with_payload(payload: serde_json::Value) -> serde_json::Value {
        let event = ApxmEvent::root(
            WarningPayload {
                code: "W999".into(),
                message: "template".into(),
            },
            EventSource::Runtime,
            "test-trace-id",
        );
        let mut raw = serde_json::to_value(event).expect("serialize event template");
        raw["payload"] = payload;
        raw
    }

    macro_rules! roundtrip_test {
        ($name:ident, $payload:expr) => {
            #[test]
            fn $name() {
                roundtrip($payload);
            }
        };
    }

    roundtrip_test!(
        serde_token,
        TokenPayload {
            text: "hello".into()
        }
    );
    roundtrip_test!(
        serde_goal_gate_verdict,
        GoalGateVerdictPayload {
            execution_id: "exec-1".into(),
            iteration: 0,
            max_iterations: 3,
            status: "needs_more".into(),
            reason: "two checks still failing".into(),
            remaining: vec!["fix auth test".into(), "rerun lint".into()],
        }
    );
    roundtrip_test!(
        serde_goal_converged,
        GoalConvergedPayload {
            execution_id: "exec-1".into(),
            iteration: 2,
            reason: "all acceptance checks pass".into(),
        }
    );
    roundtrip_test!(
        serde_goal_needs_another_pass,
        GoalNeedsAnotherPassPayload {
            execution_id: "exec-1".into(),
            iteration: 0,
            next_iteration: 1,
            reason: "gate requested another pass".into(),
        }
    );
    roundtrip_test!(
        serde_goal_halted,
        GoalHaltedPayload {
            execution_id: "exec-1".into(),
            iteration: 2,
            reason: "pass budget exhausted".into(),
            exhausted: true,
        }
    );
    roundtrip_test!(
        serde_thought,
        ThoughtPayload {
            text: "thinking...".into(),
            summary: Some("summary".into()),
        }
    );
    roundtrip_test!(
        serde_tool_call,
        ToolCallPayload {
            id: "tc-1".into(),
            name: "search".into(),
            arguments: serde_json::json!({"q": "rust"}),
        }
    );
    roundtrip_test!(
        serde_llm_done,
        LlmDonePayload {
            content: "result".into(),
            model: "gpt-4".into(),
            finish_reason: FinishReasonPayload {
                reason: "stop".into(),
            },
            usage: UsagePayload {
                input_tokens: 100,
                output_tokens: 50,
            },
            tool_calls: vec![],
            response_id: Some("resp-1".into()),
        }
    );
    roundtrip_test!(
        serde_llm_prompt,
        LlmPromptPayload {
            node_id: 1,
            node_name: Some("ask_node".into()),
            prompt: RedactedContent::from_text("private prompt"),
        }
    );
    roundtrip_test!(
        serde_usage,
        UsagePayload {
            input_tokens: 10,
            output_tokens: 20,
        }
    );
    roundtrip_test!(
        serde_retry,
        RetryPayload {
            attempt: 2,
            reason: "rate limit".into(),
            retry_after_secs: 1.5,
            backend: Some("openai".into()),
        }
    );
    roundtrip_test!(
        serde_warning,
        WarningPayload {
            code: "W001".into(),
            message: "deprecation".into(),
        }
    );
    roundtrip_test!(
        serde_citation,
        CitationPayload {
            citations: vec![Citation {
                url: Some("https://example.com".into()),
                title: Some("Example".into()),
                start_index: Some(0),
                end_index: Some(10),
            }],
        }
    );
    roundtrip_test!(
        serde_provider_event,
        ProviderEventPayload {
            provider: "anthropic".into(),
            event_type: "content_block_delta".into(),
            data: serde_json::json!({"index": 0}),
        }
    );
    roundtrip_test!(
        serde_operation_start,
        OperationStartPayload {
            node_id: 1,
            op_type: AISOperationType::Ask,
            context: None,
        }
    );
    roundtrip_test!(
        serde_operation_end,
        OperationEndPayload {
            node_id: 1,
            op_type: AISOperationType::Ask,
            duration_ms: 1234,
            success: true,
        }
    );
    roundtrip_test!(
        serde_node_output,
        NodeOutputPayload {
            node_id: 1,
            node_name: Some("output_node".into()),
            output: RedactedContent::from_json(&serde_json::json!({"status": "ok"})),
        }
    );
    roundtrip_test!(serde_node_metrics, {
        let mut metrics = NodeMetrics::new(1);
        metrics.record_operation(OperationMetric {
            node_id: 1,
            op_type: AISOperationType::ConstStr,
            duration_ms: 7,
            success: true,
        });
        NodeMetricsPayload {
            node_id: 1,
            node_name: Some("metrics_node".into()),
            metrics,
        }
    });
    roundtrip_test!(
        serde_tool_start,
        ToolStartPayload {
            name: "bash".into(),
            args: HashMap::from([("cmd".into(), serde_json::json!("ls"))]),
        }
    );
    roundtrip_test!(
        serde_tool_end,
        ToolEndPayload {
            name: "bash".into(),
            result: serde_json::json!("output"),
        }
    );
    roundtrip_test!(
        serde_plan_created,
        PlanCreatedPayload {
            plan_id: "p-1".into(),
            steps: 3,
        }
    );
    roundtrip_test!(
        serde_plan_step_started,
        PlanStepStartedPayload {
            plan_id: "p-1".into(),
            step_index: 0,
        }
    );
    roundtrip_test!(
        serde_plan_step_completed,
        PlanStepCompletedPayload {
            plan_id: "p-1".into(),
            step_index: 0,
            success: true,
        }
    );
    roundtrip_test!(
        serde_memory_read,
        MemoryReadPayload {
            scope: "short_term".into(),
            key: "last_query".into(),
        }
    );
    roundtrip_test!(
        serde_memory_write,
        MemoryWritePayload {
            scope: "long_term".into(),
            key: "user_prefs".into(),
        }
    );
    roundtrip_test!(
        serde_checkpoint_saved,
        CheckpointSavedPayload {
            checkpoint_id: "ckpt-1".into(),
        }
    );
    roundtrip_test!(
        serde_checkpoint_restored,
        CheckpointRestoredPayload {
            checkpoint_id: "ckpt-1".into(),
        }
    );
    roundtrip_test!(
        serde_scheduler_decision,
        SchedulerDecisionPayload {
            node_id: 5,
            delay_ms: 500,
            reason: "backpressure".into(),
        }
    );
    roundtrip_test!(
        serde_head_of_line_block,
        HeadOfLineBlockPayload {
            blocker_node: 2,
            blocked_node: 5,
            wait_ms: 80,
            reason: "wait_ms=80; threshold_ms=50; blocker=critical; blocked=low".into(),
        }
    );
    roundtrip_test!(
        serde_gpu_utilization,
        GpuUtilizationPayload {
            gpu_id: 0,
            utilization_pct: 75.5,
            memory_pct: 60.0,
        }
    );
    roundtrip_test!(
        serde_token_usage,
        TokenUsagePayload {
            node_id: 3,
            input_tokens: 200,
            output_tokens: 150,
        }
    );
    roundtrip_test!(serde_memoization_hit, MemoizationHitPayload { node_id: 7 });
    roundtrip_test!(
        serde_error,
        ErrorPayload {
            message: "something broke".into(),
            status: Some("500".into()),
            recoverable: false,
        }
    );
    roundtrip_test!(
        serde_context_compacted,
        ContextCompactedPayload {
            original_tokens: 10000,
            new_tokens: 5000,
        }
    );
    roundtrip_test!(
        serde_model_rerouted,
        ModelReroutedPayload {
            original_model: "gpt-4".into(),
            new_model: "gpt-3.5-turbo".into(),
            reason: "rate limit".into(),
        }
    );
    roundtrip_test!(
        serde_cancelled,
        CancelledPayload {
            reason: Some("user request".into()),
        }
    );
    roundtrip_test!(
        serde_loop_detected,
        LoopDetectedPayload {
            pattern: "A->B->A".into(),
            iterations: 3,
        }
    );
    roundtrip_test!(
        serde_context_window_warning,
        ContextWindowWarningPayload {
            current_tokens: 95000,
            max_tokens: 100000,
            utilization_pct: 95.0,
        }
    );
    roundtrip_test!(
        serde_session_start,
        SessionStartPayload {
            session_id: "sess-1".into(),
        }
    );
    roundtrip_test!(
        serde_session_end,
        SessionEndPayload {
            session_id: "sess-1".into(),
            total_turns: 5,
        }
    );
    roundtrip_test!(
        serde_turn_boundary,
        TurnBoundaryPayload {
            turn_number: 1,
            direction: TurnDirection::Request,
        }
    );

    // ── Layer 2 — agent-layer payload round-trip tests ────────────────
    roundtrip_test!(
        serde_turn_started,
        TurnStartedPayload {
            execution_id: "exec-1".into(),
            turn_id: Some("turn-1".into()),
            coordinator_label: Some("Cleo".into()),
        }
    );
    roundtrip_test!(
        serde_turn_complete,
        TurnCompletePayload {
            execution_id: "exec-1".into(),
            duration_ms: 4321,
            had_answer: true,
        }
    );
    roundtrip_test!(
        serde_turn_aborted,
        TurnAbortedPayload {
            execution_id: "exec-1".into(),
            duration_ms: 100,
            reason: "cancelled".into(),
            error_message_safe: Some("user cancelled".into()),
        }
    );
    roundtrip_test!(
        serde_subagent_spawn_begin,
        SubagentSpawnBeginPayload {
            agent_code: "crm".into(),
            agent_name: Some("CRM Module Agent".into()),
            agent_type: Some("module_agent".into()),
            module_key: Some("clic_crm".into()),
            autonomy_policy: Some("ask_before_write".into()),
            parent_span_id: Some("span-cleo".into()),
        }
    );
    roundtrip_test!(
        serde_subagent_spawn_end,
        SubagentSpawnEndPayload {
            agent_code: "crm".into(),
        }
    );
    roundtrip_test!(
        serde_subagent_llm_call_begin,
        SubagentLlmCallBeginPayload {
            agent_code: "crm".into(),
            model: "llama-3.1-8b-instruct".into(),
            backend: "vllm".into(),
            tool_manifest_count: 3,
        }
    );
    roundtrip_test!(
        serde_subagent_llm_call_end,
        SubagentLlmCallEndPayload {
            agent_code: "crm".into(),
            finish_reason: "stop".into(),
            usage: UsagePayload {
                input_tokens: 512,
                output_tokens: 128,
            },
            content_len: 640,
        }
    );
    roundtrip_test!(
        serde_tool_call_begin,
        ToolCallBeginPayload {
            agent_code: "crm".into(),
            tool_name: "crm.lead.list".into(),
            argument_keys: vec!["state".into(), "limit".into()],
        }
    );
    roundtrip_test!(
        serde_tool_call_end,
        ToolCallEndPayload {
            agent_code: "crm".into(),
            tool_name: "crm.lead.list".into(),
            result_keys: vec!["leads".into(), "next_cursor".into()],
            status: "ok".into(),
            latency_ms: 87,
        }
    );
    roundtrip_test!(
        serde_subagent_done,
        SubagentDonePayload {
            agent_code: "crm".into(),
            total_tool_calls: 2,
            usage_total: UsagePayload {
                input_tokens: 1024,
                output_tokens: 256,
            },
            evidence_excerpt: Some("3 leads in stage=qualified".into()),
        }
    );
    roundtrip_test!(
        serde_subagent_failed,
        SubagentFailedPayload {
            agent_code: "crm".into(),
            error_class: "capability_denied".into(),
            error_message_safe: "lead.write not granted".into(),
        }
    );
    roundtrip_test!(
        serde_agent_message,
        AgentMessagePayload {
            text: "Found 3 qualified leads.".into(),
            item_id: Some("item-1".into()),
            response_id: Some("resp-1".into()),
            usage: Some(UsagePayload {
                input_tokens: 1200,
                output_tokens: 80,
            }),
        }
    );
    roundtrip_test!(
        serde_approval_request,
        ApprovalRequestPayload {
            agent_code: "crm".into(),
            tool_name: "crm.lead.create".into(),
            approval_id: "appr-1".into(),
            risk_level: "medium".into(),
        }
    );
    roundtrip_test!(
        serde_approval_resolved,
        ApprovalResolvedPayload {
            approval_id: "appr-1".into(),
            decision: "approved".into(),
        }
    );

    #[test]
    fn agent_layer_kinds_are_in_core_registry() {
        for k in [
            kind::TURN_STARTED,
            kind::TURN_COMPLETE,
            kind::TURN_ABORTED,
            kind::SUBAGENT_SPAWN_BEGIN,
            kind::SUBAGENT_SPAWN_END,
            kind::SUBAGENT_LLM_CALL_BEGIN,
            kind::SUBAGENT_LLM_CALL_END,
            kind::TOOL_CALL_BEGIN,
            kind::TOOL_CALL_END,
            kind::SUBAGENT_DONE,
            kind::SUBAGENT_FAILED,
            kind::AGENT_MESSAGE,
            kind::APPROVAL_REQUEST,
            kind::APPROVAL_RESOLVED,
        ] {
            assert_eq!(kind::core_event_kind(k.name()), Some(k));
        }
    }

    #[test]
    fn core_event_kind_registry_includes_observability_events() {
        assert_eq!(
            kind::core_event_kind(kind::LLM_PROMPT.name()),
            Some(kind::LLM_PROMPT)
        );
        assert_eq!(
            kind::core_event_kind(kind::NODE_OUTPUT.name()),
            Some(kind::NODE_OUTPUT)
        );
        assert_eq!(
            kind::core_event_kind(kind::NODE_METRICS.name()),
            Some(kind::NODE_METRICS)
        );
        assert!(kind::core_event_kind("not_a_core_event").is_none());
    }

    #[test]
    fn redacted_content_omits_original_text() {
        let payload = LlmPromptPayload {
            node_id: 7,
            node_name: None,
            prompt: RedactedContent::from_text("sensitive prompt body"),
        };
        let event = ApxmEvent::root(payload, EventSource::Runtime, "trace-redacted-prompt");
        let json = serde_json::to_string(&event).expect("serialize");

        assert!(!json.contains("sensitive prompt body"));
        assert!(json.contains(REDACTION_POLICY_SUMMARY_HASH));
        assert!(json.contains(REDACTION_HASH_PREFIX_BLAKE3));
    }

    #[test]
    fn redacted_content_omits_original_json_values() {
        let payload = NodeOutputPayload {
            node_id: 7,
            node_name: None,
            output: RedactedContent::from_json(&serde_json::json!({
                "secret": "customer-token",
            })),
        };
        let event = ApxmEvent::root(payload, EventSource::Runtime, "trace-redacted-output");
        let json = serde_json::to_string(&event).expect("serialize");

        assert!(!json.contains("customer-token"));
        assert!(!json.contains("secret"));
        assert!(json.contains("object(len=1)"));
        assert!(json.contains(REDACTION_HASH_PREFIX_BLAKE3));
    }

    #[test]
    fn unknown_event_payload_deserializes_without_error() {
        let raw = raw_event_with_payload(serde_json::json!({
            "kind": "vendor_widget_updated",
            "widget_id": "w1",
            "state": { "ok": true },
        }));

        let event: ApxmEvent = serde_json::from_value(raw).expect("unknown event");

        assert_eq!(event.kind().name(), "vendor_widget_updated");
        let payload = event
            .payload
            .downcast_ref::<UnknownEventPayload>()
            .expect("unknown event payload");
        assert_eq!(payload.kind_name(), "vendor_widget_updated");
        assert_eq!(payload.payload_json()["widget_id"], "w1");
        assert_eq!(
            payload.payload_json()["state"],
            serde_json::json!({"ok": true})
        );
    }

    #[test]
    fn unknown_event_payload_roundtrips_wire_json() {
        let raw = raw_event_with_payload(serde_json::json!({
            "kind": "vendor_widget_updated",
            "widget_id": "w1",
            "state": {
                "ok": true,
                "items": [1, { "nested": "value" }],
            },
            "value": "preserved",
        }));

        let event: ApxmEvent = serde_json::from_value(raw.clone()).expect("unknown event");
        let back = serde_json::to_value(&event).expect("serialize unknown event");
        let reparsed: ApxmEvent =
            serde_json::from_value(back.clone()).expect("deserialize serialized unknown event");

        assert_eq!(back, raw);
        assert!(
            reparsed
                .payload
                .downcast_ref::<UnknownEventPayload>()
                .is_some()
        );
    }

    #[test]
    fn core_event_still_deserializes_to_typed_payload() {
        let raw = raw_event_with_payload(serde_json::json!({
            "kind": kind::OPERATION_END.name(),
            "node_id": 42,
            "op_type": "ASK",
            "duration_ms": 12,
            "success": true,
        }));

        let event: ApxmEvent = serde_json::from_value(raw).expect("core event");
        let payload = event
            .payload
            .downcast_ref::<OperationEndPayload>()
            .expect("operation end payload");

        assert_eq!(payload.node_id, 42);
        assert!(payload.success);
    }

    #[test]
    fn malformed_core_event_does_not_fallback_to_unknown() {
        let raw = raw_event_with_payload(serde_json::json!({
            "kind": kind::OPERATION_END.name(),
            "node_id": 42,
        }));

        let result = serde_json::from_value::<ApxmEvent>(raw);

        assert!(result.is_err());
    }

    #[test]
    fn registered_extension_event_deserializes_to_typed_payload() {
        let _ = register_event_payload::<TestExtensionPayload>(TEST_EXTENSION_KIND);
        let raw = raw_event_with_payload(serde_json::json!({
            "kind": TEST_EXTENSION_KIND.name(),
            "id": "abc",
            "count": 3,
        }));

        let event: ApxmEvent = serde_json::from_value(raw).expect("registered extension event");
        let payload = event
            .payload
            .downcast_ref::<TestExtensionPayload>()
            .expect("test extension payload");

        assert_eq!(payload.id, "abc");
        assert_eq!(payload.count, 3);
    }

    #[test]
    fn local_extension_registry_decodes_without_global_registration() {
        let mut registry = EventPayloadRegistry::new();
        registry
            .register_payload::<TestLocalExtensionPayload>(TEST_LOCAL_EXTENSION_KIND)
            .expect("local extension registration");
        let raw = raw_event_with_payload(serde_json::json!({
            "kind": TEST_LOCAL_EXTENSION_KIND.name(),
            "id": "local",
        }));

        let event = ApxmEvent::from_json_with_registry(raw, &registry)
            .expect("locally registered extension event");
        let payload = event
            .payload
            .downcast_ref::<TestLocalExtensionPayload>()
            .expect("local extension payload");

        assert_eq!(payload.id, "local");
    }

    #[test]
    fn local_extension_schema_error_does_not_fallback_to_unknown() {
        let mut registry = EventPayloadRegistry::new();
        registry
            .register_payload::<TestLocalExtensionMalformedPayload>(
                TEST_LOCAL_EXTENSION_MALFORMED_KIND,
            )
            .expect("local extension registration");
        let raw = raw_event_with_payload(serde_json::json!({
            "kind": TEST_LOCAL_EXTENSION_MALFORMED_KIND.name(),
            "count": "not a number",
        }));

        let result = ApxmEvent::from_json_with_registry(raw, &registry);

        assert!(result.is_err());
    }

    #[test]
    fn local_extension_registry_takes_precedence_over_global_registry() {
        let _ =
            register_event_payload::<TestGlobalPrecedencePayload>(TEST_PRECEDENCE_EXTENSION_KIND);
        let mut registry = EventPayloadRegistry::new();
        registry
            .register_payload::<TestLocalPrecedencePayload>(TEST_PRECEDENCE_EXTENSION_KIND)
            .expect("local extension registration");
        let raw = raw_event_with_payload(serde_json::json!({
            "kind": TEST_PRECEDENCE_EXTENSION_KIND.name(),
            "local_id": "local",
        }));

        let event =
            ApxmEvent::from_json_with_registry(raw, &registry).expect("local extension event");
        let payload = event
            .payload
            .downcast_ref::<TestLocalPrecedencePayload>()
            .expect("local extension payload");

        assert_eq!(payload.local_id, "local");
    }

    #[test]
    fn registered_extension_schema_error_does_not_fallback_to_unknown() {
        let _ =
            register_event_payload::<TestExtensionMalformedPayload>(TEST_EXTENSION_MALFORMED_KIND);
        let raw = raw_event_with_payload(serde_json::json!({
            "kind": TEST_EXTENSION_MALFORMED_KIND.name(),
            "id": "abc",
            "count": "not a number",
        }));

        let result = serde_json::from_value::<ApxmEvent>(raw);

        assert!(result.is_err());
    }

    #[test]
    fn registry_rejects_core_kind_registration() {
        let err = register_event_payload::<TokenPayload>(kind::TOKEN).expect_err("core kind error");

        assert!(matches!(err, EventRegistryError::CoreKind { .. }));
    }

    #[test]
    fn local_registry_rejects_core_kind_registration() {
        let mut registry = EventPayloadRegistry::new();
        let err = registry
            .register_payload::<TokenPayload>(kind::TOKEN)
            .expect_err("core kind error");

        assert!(matches!(err, EventRegistryError::CoreKind { .. }));
    }

    #[test]
    fn local_registry_rejects_duplicate_extension_kind() {
        let mut registry = EventPayloadRegistry::new();

        registry
            .register_payload::<TestLocalExtensionPayload>(TEST_LOCAL_EXTENSION_KIND)
            .expect("first local registration");
        let err = registry
            .register_payload::<TestLocalExtensionPayload>(TEST_LOCAL_EXTENSION_KIND)
            .expect_err("duplicate local registration");

        assert!(matches!(err, EventRegistryError::AlreadyRegistered { .. }));
    }

    #[test]
    fn registry_rejects_duplicate_extension_kind() {
        let first =
            register_event_payload::<TestExtensionDuplicatePayload>(TEST_EXTENSION_DUPLICATE_KIND);
        let second =
            register_event_payload::<TestExtensionDuplicatePayload>(TEST_EXTENSION_DUPLICATE_KIND);

        assert!(
            first.is_ok() || matches!(first, Err(EventRegistryError::AlreadyRegistered { .. }))
        );
        assert!(matches!(
            second,
            Err(EventRegistryError::AlreadyRegistered { .. })
        ));
    }

    #[test]
    fn global_registry_exposes_registered_kinds_sorted() {
        let _ = register_event_payload::<TestExtensionPayload>(TEST_EXTENSION_KIND);

        assert_eq!(
            registered_event_kind(TEST_EXTENSION_KIND.name()),
            Some(TEST_EXTENSION_KIND)
        );

        let names = registered_event_kinds()
            .into_iter()
            .map(|kind| kind.name())
            .collect::<Vec<_>>();
        assert!(names.windows(2).all(|pair| pair[0] <= pair[1]));
        assert!(names.contains(&TEST_EXTENSION_KIND.name()));
    }

    #[test]
    fn builder_root() {
        let event = ApxmEvent::root(
            WarningPayload {
                code: "W999".into(),
                message: "test".into(),
            },
            EventSource::Session,
            "trace-build",
        );
        assert_eq!(event.meta.seq, 0);
        assert_eq!(event.meta.trace_id, "trace-build");
        assert!(!event.meta.span_id.is_empty());
        assert!(event.meta.parent_span_id.is_none());
        assert_eq!(event.kind().name(), "warning");
        match event.meta.source {
            EventSource::Session => {}
            other => panic!("unexpected source: {other:?}"),
        }
    }

    #[test]
    fn builder_child_of() {
        let parent = ApxmEvent::root(
            TokenPayload {
                text: "parent".into(),
            },
            EventSource::Runtime,
            "trace-parent",
        );
        let parent_span = parent.meta.span_id.clone();

        let child = ApxmEvent::child_of(
            TokenPayload {
                text: "child".into(),
            },
            EventSource::Runtime,
            "trace-parent",
            &parent_span,
        );
        assert_eq!(
            child.meta.parent_span_id.as_deref(),
            Some(parent_span.as_str())
        );
        assert_ne!(child.meta.span_id, parent.meta.span_id);
    }

    #[test]
    fn builder_with_seq() {
        let event = ApxmEvent::root(
            ErrorPayload {
                message: "boom".into(),
                status: None,
                recoverable: true,
            },
            EventSource::Server,
            "trace-seq",
        )
        .with_seq(99);
        assert_eq!(event.meta.seq, 99);
    }

    #[test]
    fn builder_with_skill_provenance() {
        let event = ApxmEvent::root(
            WarningPayload {
                code: "W999".into(),
                message: "test".into(),
            },
            EventSource::Runtime,
            "trace-skill",
        )
        .with_skill_provenance(Some(SkillEventProvenance {
            skill_id: "checkout-context-triage".into(),
            skill_version: "0.1.0".into(),
            parent_skill_id: Some("parent-skill".into()),
            parent_execution_id: Some("parent-exec".into()),
            flow_name: Some("main".into()),
        }));
        let json = serde_json::to_string(&event).expect("serialize");
        let back: ApxmEvent = serde_json::from_str(&json).expect("deserialize");

        let skill = back.meta.skill.expect("skill provenance");
        assert_eq!(skill.skill_id, "checkout-context-triage");
        assert_eq!(skill.skill_version, "0.1.0");
        assert_eq!(skill.parent_skill_id.as_deref(), Some("parent-skill"));
        assert_eq!(skill.parent_execution_id.as_deref(), Some("parent-exec"));
        assert_eq!(skill.flow_name.as_deref(), Some("main"));
    }

    #[test]
    fn downcast_after_deserialize() {
        let event = ApxmEvent::root(
            OperationEndPayload {
                node_id: 42,
                op_type: AISOperationType::Ask,
                duration_ms: 12,
                success: true,
            },
            EventSource::Runtime,
            "trace-x",
        );
        let json = serde_json::to_string(&event).unwrap();
        let back: ApxmEvent = serde_json::from_str(&json).unwrap();
        let payload = back
            .payload
            .downcast_ref::<OperationEndPayload>()
            .expect("operation end payload");
        assert_eq!(payload.node_id, 42);
        assert!(payload.success);
    }

    #[test]
    fn event_source_variants_serde() {
        for source in [
            EventSource::Runtime,
            EventSource::Session,
            EventSource::Server,
            EventSource::Gui,
        ] {
            let json = serde_json::to_string(&source).unwrap();
            let _back: EventSource = serde_json::from_str(&json).unwrap();
        }

        let source = EventSource::Backend("anthropic".into());
        let json = serde_json::to_string(&source).unwrap();
        let back: EventSource = serde_json::from_str(&json).unwrap();
        match back {
            EventSource::Backend(name) => assert_eq!(name, "anthropic"),
            other => panic!("unexpected: {other:?}"),
        }

        let source = EventSource::Acp("sess-123".into());
        let json = serde_json::to_string(&source).unwrap();
        let back: EventSource = serde_json::from_str(&json).unwrap();
        match back {
            EventSource::Acp(id) => assert_eq!(id, "sess-123"),
            other => panic!("unexpected: {other:?}"),
        }
    }
}
