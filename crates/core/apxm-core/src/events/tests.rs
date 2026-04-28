//! Unit tests for the core event envelope.

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::events::event::{ApxmEvent, EventSource, SkillEventProvenance};
    use crate::events::kind;
    use crate::events::payload::*;
    use crate::types::operations::AISOperationType;
    use crate::types::{NodeMetrics, OperationMetric};

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
