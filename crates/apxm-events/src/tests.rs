//! Unit tests for the `apxm-events` crate.

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::bus::{EventBus, EventBusError};
    use crate::event::{ApxmEvent, EventSource};
    use crate::payload::*;

    // -----------------------------------------------------------------------
    // Helper: round-trip a payload through serde
    // -----------------------------------------------------------------------
    fn roundtrip(payload: EventPayload) {
        let event = ApxmEvent::new(payload, EventSource::Runtime, "test-trace-id");
        let json = serde_json::to_string(&event).expect("serialize");
        let back: ApxmEvent = serde_json::from_str(&json).expect("deserialize");
        // Basic structural check: the JSON should round-trip without panic.
        let json2 = serde_json::to_string(&back).expect("re-serialize");
        assert_eq!(json, json2);
    }

    // -----------------------------------------------------------------------
    // Serde round-trip for all 33 variants
    // -----------------------------------------------------------------------

    // -- LLM Layer (9) --

    #[test]
    fn serde_token() {
        roundtrip(EventPayload::Token(TokenPayload {
            text: "hello".into(),
        }));
    }

    #[test]
    fn serde_thought() {
        roundtrip(EventPayload::Thought(ThoughtPayload {
            text: "thinking...".into(),
            summary: Some("summary".into()),
        }));
    }

    #[test]
    fn serde_tool_call() {
        roundtrip(EventPayload::ToolCall(ToolCallPayload {
            id: "tc-1".into(),
            name: "search".into(),
            arguments: serde_json::json!({"q": "rust"}),
        }));
    }

    #[test]
    fn serde_llm_done() {
        roundtrip(EventPayload::LlmDone(LlmDonePayload {
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
        }));
    }

    #[test]
    fn serde_usage() {
        roundtrip(EventPayload::Usage(UsagePayload {
            input_tokens: 10,
            output_tokens: 20,
        }));
    }

    #[test]
    fn serde_retry() {
        roundtrip(EventPayload::Retry(RetryPayload {
            attempt: 2,
            reason: "rate limit".into(),
            retry_after_secs: 1.5,
            backend: Some("openai".into()),
        }));
    }

    #[test]
    fn serde_warning() {
        roundtrip(EventPayload::Warning(WarningPayload {
            code: "W001".into(),
            message: "deprecation".into(),
        }));
    }

    #[test]
    fn serde_citation() {
        roundtrip(EventPayload::Citation(CitationPayload {
            citations: vec![Citation {
                url: Some("https://example.com".into()),
                title: Some("Example".into()),
                start_index: Some(0),
                end_index: Some(10),
            }],
        }));
    }

    #[test]
    fn serde_provider_event() {
        roundtrip(EventPayload::ProviderEvent(ProviderEventPayload {
            provider: "anthropic".into(),
            event_type: "content_block_delta".into(),
            data: serde_json::json!({"index": 0}),
        }));
    }

    // -- Runtime Layer (16) --

    #[test]
    fn serde_operation_start() {
        roundtrip(EventPayload::OperationStart(OperationStartPayload {
            node_id: 1,
            op_type: "ASK".into(),
        }));
    }

    #[test]
    fn serde_operation_end() {
        roundtrip(EventPayload::OperationEnd(OperationEndPayload {
            node_id: 1,
            op_type: "ASK".into(),
            duration_ms: 1234,
            success: true,
        }));
    }

    #[test]
    fn serde_tool_start() {
        roundtrip(EventPayload::ToolStart(ToolStartPayload {
            name: "bash".into(),
            args: HashMap::from([("cmd".into(), serde_json::json!("ls"))]),
        }));
    }

    #[test]
    fn serde_tool_end() {
        roundtrip(EventPayload::ToolEnd(ToolEndPayload {
            name: "bash".into(),
            result: serde_json::json!("output"),
        }));
    }

    #[test]
    fn serde_plan_created() {
        roundtrip(EventPayload::PlanCreated(PlanCreatedPayload {
            plan_id: "p-1".into(),
            steps: 3,
        }));
    }

    #[test]
    fn serde_plan_step_started() {
        roundtrip(EventPayload::PlanStepStarted(PlanStepStartedPayload {
            plan_id: "p-1".into(),
            step_index: 0,
        }));
    }

    #[test]
    fn serde_plan_step_completed() {
        roundtrip(EventPayload::PlanStepCompleted(PlanStepCompletedPayload {
            plan_id: "p-1".into(),
            step_index: 0,
            success: true,
        }));
    }

    #[test]
    fn serde_memory_read() {
        roundtrip(EventPayload::MemoryRead(MemoryReadPayload {
            scope: "short_term".into(),
            key: "last_query".into(),
        }));
    }

    #[test]
    fn serde_memory_write() {
        roundtrip(EventPayload::MemoryWrite(MemoryWritePayload {
            scope: "long_term".into(),
            key: "user_prefs".into(),
        }));
    }

    #[test]
    fn serde_checkpoint_saved() {
        roundtrip(EventPayload::CheckpointSaved(CheckpointSavedPayload {
            checkpoint_id: "ckpt-1".into(),
        }));
    }

    #[test]
    fn serde_checkpoint_restored() {
        roundtrip(EventPayload::CheckpointRestored(
            CheckpointRestoredPayload {
                checkpoint_id: "ckpt-1".into(),
            },
        ));
    }

    #[test]
    fn serde_scheduler_decision() {
        roundtrip(EventPayload::SchedulerDecision(
            SchedulerDecisionPayload {
                node_id: 5,
                delay_ms: 500,
                reason: "backpressure".into(),
            },
        ));
    }

    #[test]
    fn serde_gpu_utilization() {
        roundtrip(EventPayload::GpuUtilization(GpuUtilizationPayload {
            gpu_id: 0,
            utilization_pct: 75.5,
            memory_pct: 60.0,
        }));
    }

    #[test]
    fn serde_token_usage() {
        roundtrip(EventPayload::TokenUsage(TokenUsagePayload {
            node_id: 3,
            input_tokens: 200,
            output_tokens: 150,
        }));
    }

    #[test]
    fn serde_memoization_hit() {
        roundtrip(EventPayload::MemoizationHit(MemoizationHitPayload {
            node_id: 7,
        }));
    }

    #[test]
    fn serde_error() {
        roundtrip(EventPayload::Error(ErrorPayload {
            message: "something broke".into(),
            status: Some("500".into()),
            recoverable: false,
        }));
    }

    // -- Session Layer (8) --

    #[test]
    fn serde_context_compacted() {
        roundtrip(EventPayload::ContextCompacted(ContextCompactedPayload {
            original_tokens: 10000,
            new_tokens: 5000,
        }));
    }

    #[test]
    fn serde_model_rerouted() {
        roundtrip(EventPayload::ModelRerouted(ModelReroutedPayload {
            original_model: "gpt-4".into(),
            new_model: "gpt-3.5-turbo".into(),
            reason: "rate limit".into(),
        }));
    }

    #[test]
    fn serde_cancelled() {
        roundtrip(EventPayload::Cancelled(CancelledPayload {
            reason: Some("user request".into()),
        }));
    }

    #[test]
    fn serde_loop_detected() {
        roundtrip(EventPayload::LoopDetected(LoopDetectedPayload {
            pattern: "A->B->A".into(),
            iterations: 3,
        }));
    }

    #[test]
    fn serde_context_window_warning() {
        roundtrip(EventPayload::ContextWindowWarning(
            ContextWindowWarningPayload {
                current_tokens: 95000,
                max_tokens: 100000,
                utilization_pct: 95.0,
            },
        ));
    }

    #[test]
    fn serde_session_start() {
        roundtrip(EventPayload::SessionStart(SessionStartPayload {
            session_id: "sess-1".into(),
        }));
    }

    #[test]
    fn serde_session_end() {
        roundtrip(EventPayload::SessionEnd(SessionEndPayload {
            session_id: "sess-1".into(),
            total_turns: 5,
        }));
    }

    #[test]
    fn serde_turn_boundary() {
        roundtrip(EventPayload::TurnBoundary(TurnBoundaryPayload {
            turn_number: 1,
            direction: TurnDirection::Request,
        }));
    }

    // -----------------------------------------------------------------------
    // EventBus tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn bus_publish_subscribe() {
        let bus = EventBus::new();
        let mut sub = bus.subscribe();

        let event = ApxmEvent::new(
            EventPayload::Token(TokenPayload {
                text: "hi".into(),
            }),
            EventSource::Backend("openai".into()),
            "trace-1",
        )
        .with_seq(42);

        bus.publish(event).expect("publish should succeed");

        let received = sub.recv().await.expect("should receive event");
        assert_eq!(received.meta.seq, 42);
        assert_eq!(received.meta.trace_id, "trace-1");

        match &received.payload {
            EventPayload::Token(t) => assert_eq!(t.text, "hi"),
            other => panic!("unexpected payload: {other:?}"),
        }
    }

    #[tokio::test]
    async fn bus_lagged_handling() {
        // Capacity of 2: third publish should cause lag when subscriber reads.
        let bus = EventBus::with_capacity(2);
        let mut sub = bus.subscribe();

        for i in 0..4 {
            let event = ApxmEvent::new(
                EventPayload::Token(TokenPayload {
                    text: format!("tok-{i}"),
                }),
                EventSource::Runtime,
                "trace-lag",
            )
            .with_seq(i);
            let _ = bus.publish(event);
        }

        // The subscriber should report a lag.
        match sub.recv().await {
            Err(EventBusError::Lagged(n)) => {
                assert!(n > 0, "should have lagged by at least 1 event");
            }
            Ok(event) => {
                // Depending on timing, we might get a later event.
                // That's acceptable too — the important thing is no panic.
                assert!(event.meta.seq >= 2);
            }
            Err(EventBusError::Closed) => panic!("bus should not be closed"),
        }
    }

    #[tokio::test]
    async fn bus_closed_handling() {
        let bus = EventBus::new();
        let mut sub = bus.subscribe();
        drop(bus);

        match sub.recv().await {
            Err(EventBusError::Closed) => {} // expected
            other => panic!("expected Closed, got {other:?}"),
        }
    }

    #[test]
    fn bus_subscriber_count() {
        let bus = EventBus::new();
        assert_eq!(bus.subscriber_count(), 0);

        let _s1 = bus.subscribe();
        assert_eq!(bus.subscriber_count(), 1);

        let _s2 = bus.subscribe();
        assert_eq!(bus.subscriber_count(), 2);

        drop(_s1);
        assert_eq!(bus.subscriber_count(), 1);
    }

    #[test]
    fn bus_publish_no_subscribers() {
        let bus = EventBus::new();
        let event = ApxmEvent::new(
            EventPayload::Token(TokenPayload {
                text: "orphan".into(),
            }),
            EventSource::Runtime,
            "trace-orphan",
        );
        // Should return Err with the event when no subscribers.
        let result = bus.publish(event);
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // Builder tests
    // -----------------------------------------------------------------------

    #[test]
    fn builder_new() {
        let event = ApxmEvent::new(
            EventPayload::Warning(WarningPayload {
                code: "W999".into(),
                message: "test".into(),
            }),
            EventSource::Session,
            "trace-build",
        );
        assert_eq!(event.meta.seq, 0);
        assert_eq!(event.meta.trace_id, "trace-build");
        match event.meta.source {
            EventSource::Session => {}
            other => panic!("unexpected source: {other:?}"),
        }
    }

    #[test]
    fn builder_with_seq() {
        let event = ApxmEvent::new(
            EventPayload::Error(ErrorPayload {
                message: "boom".into(),
                status: None,
                recoverable: true,
            }),
            EventSource::Server,
            "trace-seq",
        )
        .with_seq(99);
        assert_eq!(event.meta.seq, 99);
    }

    // -----------------------------------------------------------------------
    // EventSource serde
    // -----------------------------------------------------------------------

    #[test]
    fn event_source_backend_serde() {
        let source = EventSource::Backend("anthropic".into());
        let json = serde_json::to_string(&source).unwrap();
        let back: EventSource = serde_json::from_str(&json).unwrap();
        match back {
            EventSource::Backend(name) => assert_eq!(name, "anthropic"),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn event_source_variants_serde() {
        for source in [
            EventSource::Runtime,
            EventSource::Session,
            EventSource::Server,
        ] {
            let json = serde_json::to_string(&source).unwrap();
            let _back: EventSource = serde_json::from_str(&json).unwrap();
        }
    }
}
