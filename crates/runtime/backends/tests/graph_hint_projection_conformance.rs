//! Conformance: every graph-hint projector, run against the published hint
//! vectors.
//!
//! Adapter conformance lives here rather than in `contracts/vectors/` on
//! purpose. A published vector states a verdict about a *document* and is read
//! by every language that implements the contract. What an adapter projects is
//! not a document: it is a per-binding decision expressed in provider mechanism
//! names (`cache_prompt`, `vllm_xargs`, the provider priority key) that were
//! deliberately moved out of the common crates and are guarded from returning
//! there. Publishing them as contract vectors would put them straight back into
//! the shared surface, and would recreate the provider-pinned vector catalogue
//! ADR 0021 retired.
//!
//! So the *input* is the published vector file — the same envelopes the schema
//! and the decode path are held against — and the *assertions* are the
//! projector contract itself, applied to every adapter that implements it. An
//! adapter that leaks an unsupported field, renegotiates a plan between
//! attempts, projects nondeterministically, disguises an unsupported field as a
//! profile choice, or accepts an envelope the contract rejects fails here.
//!
//! `apxm-backends` had no integration test target before this file; each
//! adapter tested itself from the inside. A projector contract is a statement
//! about adapters as a set, so it is stated once, from outside, over all of
//! them.

use std::collections::BTreeSet;
use std::path::PathBuf;

use apxm_backends::llm::backends::{GraphAwareVllmBackend, LlamaCppBackend, MockLLMBackend};
use apxm_core::constants::llm::apxm::graph_hints as hint_keys;
use apxm_core::types::{
    ApxmGraphHints, GraphHintCapabilities, GraphHintDispatchProjection, GraphHintField,
    GraphHintFieldCapability, GraphHintProjector, ProjectionOutcome,
};
use serde_json::Value;

// ---------------------------------------------------------------------------
// Vector loading
// ---------------------------------------------------------------------------

const VECTORS: &str = "apxm.inference-graph-hints.json";

struct Vector {
    name: String,
    input: Value,
    expected_valid: bool,
}

fn load_vectors() -> Vec<Vector> {
    // crates/runtime/backends -> agents repo root
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .join("contracts/vectors")
        .join(VECTORS);
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let doc: Value =
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()));
    doc.as_array()
        .expect("vectors file is a JSON array")
        .iter()
        .map(|entry| Vector {
            name: entry["name"].as_str().expect("vector name").to_owned(),
            input: entry["input"].clone(),
            expected_valid: entry["expected_valid"].as_bool().expect("expected_valid"),
        })
        .collect()
}

/// The admission path a projector takes before it renders anything.
fn admit(document: &Value) -> Result<ApxmGraphHints, String> {
    let hints: ApxmGraphHints =
        serde_json::from_value(document.clone()).map_err(|e| e.to_string())?;
    hints.validate().map(|()| hints)
}

fn admitted_vectors() -> Vec<(String, ApxmGraphHints)> {
    load_vectors()
        .into_iter()
        .filter(|vector| vector.expected_valid)
        .map(|vector| {
            let hints = admit(&vector.input).expect("an admitted vector decodes");
            (vector.name, hints)
        })
        .collect()
}

// ---------------------------------------------------------------------------
// The adapters under test
// ---------------------------------------------------------------------------

fn config() -> Option<Value> {
    Some(serde_json::json!({
        "model": "deployment-model",
        "base_url": "https://provider.example.test/v1",
    }))
}

/// Every adapter in this crate that implements the projector, boxed behind the
/// trait so the contract is stated once. Construction is configuration only —
/// no adapter here contacts a provider.
async fn projectors() -> Vec<(&'static str, Box<dyn GraphHintProjector>)> {
    vec![
        (
            "mock",
            Box::new(MockLLMBackend::new()) as Box<dyn GraphHintProjector>,
        ),
        (
            "llama.cpp",
            Box::new(
                LlamaCppBackend::new("", config())
                    .await
                    .expect("configured llama.cpp backend"),
            ),
        ),
        (
            "vllm",
            Box::new(
                GraphAwareVllmBackend::new("", config())
                    .await
                    .expect("configured vLLM backend"),
            ),
        ),
    ]
}

/// The common wire name each field takes inside the projected envelope. This
/// is stated here, not imported from the renderer, so the test can disagree
/// with it: a renderer that starts emitting an omitted field under its
/// documented key is caught rather than followed.
fn wire_key(field: GraphHintField) -> &'static str {
    match field {
        GraphHintField::Scope => hint_keys::SCOPE,
        GraphHintField::CriticalPath => hint_keys::CRITICAL_PATH,
        GraphHintField::SuccessorRefs => hint_keys::SUCCESSOR_REFS,
        GraphHintField::RemainingPathLen => hint_keys::REMAINING_PATH_LEN,
        GraphHintField::StageIndex => hint_keys::STAGE_INDEX,
        GraphHintField::WorkClass => hint_keys::WORK_CLASS,
        GraphHintField::EstimatedInputTokens => hint_keys::ESTIMATED_INPUT_TOKENS,
        GraphHintField::EstimatedOutputTokens => hint_keys::ESTIMATED_OUTPUT_TOKENS,
        GraphHintField::ExpectedSharedPrefixTokens => hint_keys::EXPECTED_SHARED_PREFIX_TOKENS,
        GraphHintField::PrefixWarmupEligible => hint_keys::PREFIX_WARMUP_ELIGIBLE,
        GraphHintField::PipelineEligible => hint_keys::PIPELINE_ELIGIBLE,
        GraphHintField::CoexecutionGroupRef => hint_keys::COEXECUTION_GROUP_REF,
        GraphHintField::Objective => hint_keys::OBJECTIVE,
        // The preference is the required member of a reusable-context intent,
        // so the container is what an adapter that cannot carry it must omit.
        GraphHintField::ReusePreference => hint_keys::REUSABLE_CONTEXT,
        GraphHintField::AffinityRef => hint_keys::AFFINITY_REF,
        GraphHintField::BenefitHorizonMs => hint_keys::BENEFIT_HORIZON_MS,
        GraphHintField::ExpectedUses => hint_keys::EXPECTED_USES,
    }
}

fn rendered(projected: &GraphHintDispatchProjection) -> String {
    Value::Object(projected.provider_fields.clone()).to_string()
}

// ---------------------------------------------------------------------------
// The projector contract
// ---------------------------------------------------------------------------

/// Every present envelope gets a complete field-by-field report, and the report
/// is the one its own declared capabilities admit.
#[tokio::test]
async fn every_admitted_vector_gets_a_complete_report_from_every_adapter() {
    for (adapter, projector) in projectors().await {
        let capabilities = projector.graph_hint_capabilities();
        for (name, hints) in admitted_vectors() {
            let projected = projector
                .project_graph_hints(Some(&hints), 0)
                .unwrap_or_else(|e| panic!("{adapter} refused admitted vector '{name}': {e}"));

            assert_eq!(
                projected.plan.outcomes.len(),
                GraphHintField::ALL.len(),
                "{adapter} left a field out of its report for '{name}'",
            );
            assert_eq!(
                projected.plan.capability_digest,
                capabilities.digest(),
                "{adapter} planned against a capability surface it does not declare",
            );
            assert_eq!(
                projected.plan.graph_hints_digest.as_deref(),
                Some(hints.digest().as_str()),
                "{adapter} committed to a different envelope than it was given",
            );
            projected
                .plan
                .validate_against(&capabilities)
                .unwrap_or_else(|e| panic!("{adapter} plan for '{name}' is not admissible: {e}"));
        }
    }
}

/// The exit gate the whole design exists for: a field an adapter does not
/// support never reaches a provider request, whatever the envelope says.
#[tokio::test]
async fn an_unprojected_field_never_reaches_the_provider_request() {
    for (adapter, projector) in projectors().await {
        for (name, hints) in admitted_vectors() {
            let projected = projector
                .project_graph_hints(Some(&hints), 0)
                .expect("admitted vector projects");
            let body = rendered(&projected);

            for field in GraphHintField::ALL {
                if projected.plan.projects(*field) {
                    continue;
                }
                assert!(
                    !body.contains(wire_key(*field)),
                    "{adapter} leaked {field:?} into the provider request for '{name}'",
                );
            }
        }
    }
}

/// A binding may only send what it declares. The declared set is the ceiling;
/// a profile may lower it, nothing may raise it.
#[tokio::test]
async fn what_reaches_the_request_is_within_the_declared_capability() {
    for (adapter, projector) in projectors().await {
        let capabilities = projector.graph_hint_capabilities();
        let declared: BTreeSet<GraphHintField> = GraphHintField::ALL
            .iter()
            .copied()
            .filter(|field| capabilities.supports(*field))
            .collect();

        for (name, hints) in admitted_vectors() {
            let projected = projector
                .project_graph_hints(Some(&hints), 0)
                .expect("admitted vector projects");
            let sent: BTreeSet<GraphHintField> = GraphHintField::ALL
                .iter()
                .copied()
                .filter(|field| projected.plan.projects(*field))
                .collect();
            assert!(
                sent.is_subset(&declared),
                "{adapter} sent {:?} beyond its declaration for '{name}'",
                sent.difference(&declared).collect::<Vec<_>>(),
            );
        }
    }
}

/// An unsupported field is reported as unsupported, not dressed up as a
/// profile decision. `OmittedByProfile` means a mechanism exists and was
/// withheld; a binding with no mechanism has nothing to withhold.
///
/// Stated for the ambient profile these adapters run under here: an isolation
/// or control arm is a separate configuration and states its own withholding.
#[tokio::test]
async fn an_unsupported_field_is_never_reported_as_a_profile_choice() {
    for (adapter, projector) in projectors().await {
        let capabilities = projector.graph_hint_capabilities();
        for (name, hints) in admitted_vectors() {
            let projected = projector
                .project_graph_hints(Some(&hints), 0)
                .expect("admitted vector projects");
            for field in GraphHintField::ALL {
                if capabilities.supports(*field) {
                    continue;
                }
                assert_eq!(
                    projected.plan.outcomes.get(field),
                    Some(&ProjectionOutcome::OmittedUnsupported),
                    "{adapter} reported unsupported {field:?} as something else for '{name}'",
                );
            }
        }
    }
}

/// Planning is pure. The same envelope through the same binding is the same
/// plan, the same rendered request, and the same digests.
#[tokio::test]
async fn projection_is_deterministic() {
    for (adapter, projector) in projectors().await {
        for (name, hints) in admitted_vectors() {
            let first = projector
                .project_graph_hints(Some(&hints), 0)
                .expect("admitted vector projects");
            let second = projector
                .project_graph_hints(Some(&hints), 0)
                .expect("admitted vector projects");
            assert_eq!(
                first, second,
                "{adapter} projected '{name}' two different ways",
            );
        }
    }
}

/// Attempts do not renegotiate semantics: a retry carries the same plan and
/// the same projected request. Only the attempt ordinal moves.
#[tokio::test]
async fn a_retry_carries_the_same_plan_and_the_same_request() {
    for (adapter, projector) in projectors().await {
        for (name, hints) in admitted_vectors() {
            let first = projector
                .project_graph_hints(Some(&hints), 0)
                .expect("admitted vector projects");
            let retry = projector
                .project_graph_hints(Some(&hints), 1)
                .expect("admitted vector projects on retry");

            assert_eq!(
                first.plan, retry.plan,
                "{adapter} changed its plan between attempts of '{name}'",
            );
            assert_eq!(
                first.provider_fields, retry.provider_fields,
                "{adapter} changed the provider request between attempts of '{name}'",
            );
            assert_eq!(
                first.projection.projected_request_digest,
                retry.projection.projected_request_digest,
                "{adapter} changed the projected request digest between attempts of '{name}'",
            );
            assert_eq!(first.projection.attempt, 0);
            assert_eq!(retry.projection.attempt, 1);
        }
    }
}

/// Projection evidence is digests and closed vocabulary. The correlation
/// references the runtime supplied are in the envelope, not in the record of
/// what happened to it.
#[tokio::test]
async fn projection_evidence_carries_no_envelope_content() {
    for (adapter, projector) in projectors().await {
        for (name, hints) in admitted_vectors() {
            let projected = projector
                .project_graph_hints(Some(&hints), 0)
                .expect("admitted vector projects");
            let evidence = projected.to_evidence_json().to_string();

            let mut references = vec![
                hints.scope.graph_ref.clone(),
                hints.scope.graph_execution_ref.clone(),
                hints.scope.node_ref.clone(),
                hints.scope.node_execution_ref.clone(),
            ];
            references.extend(hints.facts.successor_refs.iter().cloned());
            if let Some(reuse) = &hints.intents.reusable_context {
                references.extend(reuse.affinity_ref.clone());
            }
            for reference in references {
                assert!(
                    !evidence.contains(&reference),
                    "{adapter} evidence for '{name}' carries the reference {reference}",
                );
            }
        }
    }
}

/// A rejected envelope is rejected by every adapter, and rejection happens
/// before any provider field exists.
#[tokio::test]
async fn no_adapter_projects_a_rejected_envelope() {
    let rejected: Vec<(String, ApxmGraphHints)> = load_vectors()
        .into_iter()
        .filter(|vector| !vector.expected_valid)
        .filter_map(|vector| {
            // A vector the closed type cannot even decode is already refused;
            // the interesting ones are those that decode and must still fail.
            serde_json::from_value::<ApxmGraphHints>(vector.input.clone())
                .ok()
                .map(|hints| (vector.name, hints))
        })
        .collect();
    assert!(
        !rejected.is_empty(),
        "the vector file states no rejection the type can hold",
    );

    for (adapter, projector) in projectors().await {
        for (name, hints) in &rejected {
            assert!(
                projector.project_graph_hints(Some(hints), 0).is_err(),
                "{adapter} projected the rejected envelope '{name}'",
            );
        }
    }
}

// ---------------------------------------------------------------------------
// What each binding declares
// ---------------------------------------------------------------------------

/// The declarations themselves, held exactly. A binding that quietly widens
/// what it claims to carry changes its capability digest and would be caught
/// before send; this states what the digest is currently over.
#[tokio::test]
async fn each_binding_declares_exactly_the_fields_it_carries() {
    let expected: &[(&str, &[GraphHintField], GraphHintFieldCapability)] = &[
        (
            "mock",
            &[],
            // Nothing is declared, so the kind is unused.
            GraphHintFieldCapability::Unsupported,
        ),
        (
            "llama.cpp",
            &[GraphHintField::Scope, GraphHintField::ReusePreference],
            GraphHintFieldCapability::Direct,
        ),
        (
            "vllm",
            &[
                GraphHintField::Scope,
                GraphHintField::CriticalPath,
                GraphHintField::SuccessorRefs,
                GraphHintField::RemainingPathLen,
                GraphHintField::StageIndex,
                GraphHintField::Objective,
                GraphHintField::ReusePreference,
                GraphHintField::AffinityRef,
                GraphHintField::BenefitHorizonMs,
            ],
            GraphHintFieldCapability::Derived,
        ),
    ];

    for (adapter, projector) in projectors().await {
        let (_, fields, kind) = expected
            .iter()
            .find(|(name, _, _)| *name == adapter)
            .unwrap_or_else(|| panic!("{adapter} has no declared expectation"));
        let capabilities = projector.graph_hint_capabilities();
        let declared: BTreeSet<GraphHintField> = GraphHintField::ALL
            .iter()
            .copied()
            .filter(|field| capabilities.supports(*field))
            .collect();
        assert_eq!(
            declared,
            fields.iter().copied().collect::<BTreeSet<_>>(),
            "{adapter} declares a different field set than this contract records",
        );
        for field in *fields {
            assert_eq!(
                capabilities.fields.get(field),
                Some(kind),
                "{adapter} carries {field:?} by a different means than recorded",
            );
        }
    }
}

/// A zero-capability binding is conforming, and it is the floor: it reports
/// every field and sends nothing.
#[tokio::test]
async fn the_zero_capability_binding_is_the_floor() {
    let mock = MockLLMBackend::new();
    assert_eq!(
        mock.graph_hint_capabilities(),
        GraphHintCapabilities::none(),
    );
    for (name, hints) in admitted_vectors() {
        let projected = mock
            .project_graph_hints(Some(&hints), 0)
            .expect("zero capability is a conforming projection");
        assert!(
            projected.provider_fields.is_empty(),
            "the zero-capability binding rendered a field for '{name}'",
        );
        assert!(
            projected
                .plan
                .outcomes
                .values()
                .all(|outcome| *outcome == ProjectionOutcome::OmittedUnsupported)
        );
    }
}
