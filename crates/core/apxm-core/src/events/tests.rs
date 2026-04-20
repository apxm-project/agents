//! Unit tests for the core event envelope.

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::events::event::{ApxmEvent, EventSource};
    use crate::events::payload::*;

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
            op_type: "ASK".into(),
        }
    );
    roundtrip_test!(
        serde_operation_end,
        OperationEndPayload {
            node_id: 1,
            op_type: "ASK".into(),
            duration_ms: 1234,
            success: true,
        }
    );
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
        assert_eq!(child.meta.parent_span_id.as_deref(), Some(parent_span.as_str()));
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
    fn downcast_after_deserialize() {
        let event = ApxmEvent::root(
            OperationEndPayload {
                node_id: 42,
                op_type: "ASK".into(),
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
