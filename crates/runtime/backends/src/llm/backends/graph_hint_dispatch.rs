//! Adapter-side seam between one graph-hint projection and one dispatch.
//!
//! Projection evidence is digests and closed vocabulary. The provider request
//! body, its headers, and its prompt content never enter this record.

use super::response::LLMResponse;
use super::traits::StreamChunk;
use apxm_core::constants::llm::apxm::graph_hints as hint_keys;
use apxm_core::types::GraphHintDispatchProjection;
use std::pin::Pin;
use tokio_stream::{Stream, StreamExt};

/// Attach the adapter's own evidence to one response: what it planned field by
/// field, and what it projected for this attempt.
///
/// Provider acknowledgement and outcome measurement are separate claims, and
/// no provider response an adapter here parses states either, so neither is
/// recorded. A response-side evidence layer arrives with the first provider
/// contract that actually reports one.
pub fn record_graph_hint_evidence(
    mut response: LLMResponse,
    projected: Option<&GraphHintDispatchProjection>,
) -> LLMResponse {
    let Some(projected) = projected else {
        return response;
    };
    if projected.plan.outcomes.is_empty() {
        return response;
    }
    for (key, value) in [
        (
            hint_keys::PLAN,
            serde_json::to_value(&projected.plan).unwrap_or_default(),
        ),
        (
            hint_keys::PROJECTION,
            serde_json::to_value(&projected.projection).unwrap_or_default(),
        ),
    ] {
        response.metadata.insert(key.to_owned(), value);
    }
    response
}

/// The streaming form: the same evidence lands on the terminal response.
pub fn stream_with_graph_hint_evidence<'a>(
    inner: Pin<Box<dyn Stream<Item = anyhow::Result<StreamChunk>> + Send + 'a>>,
    projected: Option<GraphHintDispatchProjection>,
) -> Pin<Box<dyn Stream<Item = anyhow::Result<StreamChunk>> + Send + 'a>> {
    Box::pin(inner.map(move |chunk| match chunk {
        Ok(StreamChunk::Done(response)) => Ok(StreamChunk::Done(record_graph_hint_evidence(
            response,
            projected.as_ref(),
        ))),
        other => other,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::backends::llama_cpp::LlamaCppBackend;
    use crate::llm::backends::request::LLMRequest;
    use crate::llm::backends::vllm::GraphAwareVllmBackend;
    use apxm_core::types::{
        ApxmGraphHints, GraphHintCapabilities, GraphHintField, GraphHintProjector, NodeGraphFacts,
        OptimizationObjective, ProjectionOutcome, ReusableContextIntent, ReusePreference,
        WorkClass,
    };
    use serde_json::Value;

    /// Hints that state something in every field family, so an adapter that
    /// leaks an unsupported field has something to leak.
    fn full_hints() -> ApxmGraphHints {
        let mut hints = ApxmGraphHints::critical_path(
            "graph.1",
            "graph-execution.1",
            "node.1",
            "node-execution.1",
            vec!["node.2".into()],
        );
        hints.facts = NodeGraphFacts {
            critical_path: Some(true),
            successor_refs: vec!["node.2".into()],
            remaining_path_len: Some(3),
            stage_index: Some(1),
            work_class: Some(WorkClass::Long),
            estimated_input_tokens: Some(4096),
            estimated_output_tokens: Some(256),
            expected_shared_prefix_tokens: Some(2048),
            prefix_warmup_eligible: Some(true),
            pipeline_eligible: Some(true),
            coexecution_group_ref: Some("coexecution-group.1".into()),
        };
        hints.intents.objective = Some(OptimizationObjective::Balanced);
        hints.intents.reusable_context = Some(ReusableContextIntent {
            preference: ReusePreference::PreferWhenBeneficial,
            affinity_ref: Some("affinity.1".into()),
            benefit_horizon_ms: Some(30_000),
            expected_uses: Some(4),
        });
        hints.validate().expect("the fixture is admissible");
        hints
    }

    fn config(model: &str) -> Option<Value> {
        Some(serde_json::json!({
            "model": model,
            "base_url": "https://provider.example.test/v1",
        }))
    }

    fn request_with_hints() -> LLMRequest {
        let mut request = LLMRequest::new("status");
        request.model = Some("deployment-model".to_string());
        request.apxm_hints = Some(full_hints());
        request
    }

    /// A projector with no declared capabilities: the exit gate for Phase B.
    struct ZeroCapabilityAdapter;
    impl GraphHintProjector for ZeroCapabilityAdapter {}

    #[test]
    fn a_zero_capability_adapter_reports_everything_and_sends_nothing() {
        let hints = full_hints();
        let projected = ZeroCapabilityAdapter
            .project_graph_hints(Some(&hints), 0)
            .expect("zero capability is a conforming projection");

        assert_eq!(projected.plan.outcomes.len(), GraphHintField::ALL.len());
        for field in GraphHintField::ALL {
            assert_eq!(
                projected.plan.outcomes.get(field),
                Some(&ProjectionOutcome::OmittedUnsupported),
                "field {field:?} is missing from the projection report",
            );
        }
        assert!(projected.provider_fields.is_empty());
        assert_eq!(
            projected.plan.capability_digest,
            GraphHintCapabilities::none().digest(),
        );
    }

    #[tokio::test]
    async fn a_zero_capability_backend_receives_an_unchanged_request() {
        // The generic provider adapter declares no graph capabilities, so the
        // request it dispatches with hints attached is the request it would
        // have dispatched without them.
        let backend = crate::llm::backends::mock::MockLLMBackend::new();
        assert_eq!(
            backend.graph_hint_capabilities(),
            GraphHintCapabilities::none(),
        );
        let projected = backend
            .project_graph_hints(Some(&full_hints()), 0)
            .expect("projection");
        assert!(projected.provider_fields.is_empty());

        // And the request really goes through an adapter's injection path: with
        // no field authorized, what comes out of `inject_hints` is what went in.
        let injector = LlamaCppBackend::new("", config("deployment-model"))
            .await
            .expect("configured llama.cpp backend");
        let mut unhinted = request_with_hints();
        unhinted.apxm_hints = None;
        let before = unhinted.extra_body.clone();
        let (dispatched, projection) = injector.inject_hints(unhinted, 0).expect("injection");
        assert!(
            projection
                .expect("an injection always reports its plan")
                .provider_fields
                .is_empty(),
            "a request with no hints authorized a provider field",
        );
        assert_eq!(dispatched.extra_body, before);
    }

    #[tokio::test]
    async fn llama_cpp_sends_only_the_two_fields_it_supports() {
        let backend = LlamaCppBackend::new("", config("deployment-model"))
            .await
            .expect("configured llama.cpp backend");
        let (request, projected) = backend
            .inject_hints(request_with_hints(), 0)
            .expect("projection");
        let projected = projected.expect("hints were present");
        let extra = request.extra_body.expect("extra body");

        assert_eq!(extra["cache_prompt"], Value::Bool(true));
        let envelope = &extra["apxm"];
        assert!(envelope.get("scope").is_some());
        let rendered = envelope.to_string();
        for absent in [
            "successor_refs",
            "critical_path",
            "estimated_input_tokens",
            "expected_shared_prefix_tokens",
            "objective",
            "affinity_ref",
            "benefit_horizon_ms",
            "coexecution_group_ref",
            "work_class",
        ] {
            assert!(
                !rendered.contains(absent),
                "unsupported field {absent} reached the llama.cpp request",
            );
        }
        // The provider request never carries a slot coordinate.
        assert!(!extra.to_string().contains("id_slot"));

        for unsupported in [
            GraphHintField::CriticalPath,
            GraphHintField::SuccessorRefs,
            GraphHintField::AffinityRef,
            GraphHintField::BenefitHorizonMs,
            GraphHintField::PrefixWarmupEligible,
            GraphHintField::PipelineEligible,
        ] {
            assert_eq!(
                projected.plan.outcomes.get(&unsupported),
                Some(&ProjectionOutcome::OmittedUnsupported),
                "{unsupported:?} must report an explicit omission",
            );
        }
    }

    #[tokio::test]
    async fn vllm_sends_the_envelope_it_declares_and_nothing_else() {
        let backend = GraphAwareVllmBackend::new("", config("deployment-model"))
            .await
            .expect("configured vLLM backend");
        let (request, projected) = backend
            .inject_hints(request_with_hints(), 0)
            .expect("projection");
        let projected = projected.expect("hints were present");
        let extra = request.extra_body.expect("extra body");
        let envelope = &extra["vllm_xargs"]["apxm"];

        assert!(envelope.get("scope").is_some());
        assert!(envelope["facts"].get("successor_refs").is_some());
        let rendered = envelope.to_string();
        for absent in [
            "estimated_input_tokens",
            "estimated_output_tokens",
            "expected_shared_prefix_tokens",
            "prefix_warmup_eligible",
            "pipeline_eligible",
            "coexecution_group_ref",
            "work_class",
            "expected_uses",
        ] {
            assert!(
                !rendered.contains(absent),
                "unsupported field {absent} reached the vLLM request",
            );
        }

        // No scheduler policy has been observed, so the queue value this
        // binding would derive from critical-path work is not admitted and the
        // request carries no priority at all.
        assert!(extra.get("priority").is_none());
        assert_eq!(
            projected.plan.outcomes.get(&GraphHintField::CriticalPath),
            Some(&ProjectionOutcome::OmittedByProfile {
                reason: apxm_core::types::ReasonCode::MechanismNotAdmitted,
            }),
        );
    }

    #[tokio::test]
    async fn projection_evidence_carries_digests_and_no_provider_body() {
        let backend = LlamaCppBackend::new("", config("deployment-model"))
            .await
            .expect("configured llama.cpp backend");
        let (_, projected) = backend
            .inject_hints(request_with_hints(), 0)
            .expect("projection");
        let projected = projected.expect("hints were present");
        let response = record_graph_hint_evidence(
            crate::llm::backends::LLMResponse::new(
                "answer",
                "deployment-model",
                Default::default(),
                apxm_core::types::FinishReason::Stop,
            ),
            Some(&projected),
        );

        let plan = &response.metadata[hint_keys::PLAN];
        let projection = &response.metadata[hint_keys::PROJECTION];
        assert!(
            plan["graph_hints_digest"]
                .as_str()
                .expect("hint digest")
                .starts_with("sha256:")
        );
        assert!(
            projection["projected_request_digest"]
                .as_str()
                .expect("request digest")
                .starts_with("sha256:")
        );
        // Only the adapter's own claims are recorded: what it planned per
        // field, and what it sent. Nothing claims the provider agreed.
        assert!(
            projected
                .plan
                .outcomes
                .get(&GraphHintField::Scope)
                .is_some_and(ProjectionOutcome::is_projected)
        );
        assert_eq!(response.metadata.len(), 2);

        // Mechanism identifiers are evidence labels and belong here; prompt
        // content, generated content, endpoints, and the provider body do not.
        let rendered = serde_json::to_string(&response.metadata).expect("evidence serializes");
        assert!(rendered.contains("llama.cache_prompt"));
        for forbidden in ["status", "answer", "base_url", "api_key", "extra_body"] {
            assert!(
                !rendered.contains(forbidden),
                "projection evidence leaked {forbidden}",
            );
        }
    }

    /// The streaming form carries the same evidence. A stream that dropped it
    /// would leave a dispatch with no record of what it projected, and only
    /// the terminal chunk is the response — the token chunks stay untouched.
    #[tokio::test]
    async fn a_streamed_dispatch_carries_the_same_evidence_on_its_terminal_chunk() {
        let backend = LlamaCppBackend::new("", config("deployment-model"))
            .await
            .expect("configured llama.cpp backend");
        let (_, projected) = backend
            .inject_hints(request_with_hints(), 0)
            .expect("projection");
        let projected = projected.expect("hints were present");
        let buffered = record_graph_hint_evidence(
            crate::llm::backends::LLMResponse::new(
                "answer",
                "deployment-model",
                Default::default(),
                apxm_core::types::FinishReason::Stop,
            ),
            Some(&projected),
        );

        let inner: Vec<anyhow::Result<StreamChunk>> = vec![
            Ok(StreamChunk::Token("an".into())),
            Ok(StreamChunk::Done(crate::llm::backends::LLMResponse::new(
                "answer",
                "deployment-model",
                Default::default(),
                apxm_core::types::FinishReason::Stop,
            ))),
        ];
        let chunks: Vec<StreamChunk> = stream_with_graph_hint_evidence(
            Box::pin(tokio_stream::iter(inner)),
            Some(projected.clone()),
        )
        .map(|chunk| chunk.expect("chunk"))
        .collect()
        .await;

        assert!(matches!(chunks[0], StreamChunk::Token(_)));
        let StreamChunk::Done(ref streamed) = chunks[1] else {
            panic!("the last chunk is the response");
        };
        assert_eq!(streamed.metadata, buffered.metadata);
    }
}
