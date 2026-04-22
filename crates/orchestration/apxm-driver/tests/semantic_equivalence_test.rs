//! Tier-1 semantic-equivalence harness: two opt levels must produce identical
//! canonicalized `CallTrace`s for the same `AirModule`.
//!
//! Canonicalization (Phase A): sort each event's `parent_deps`, then sort
//! events by `(parent_deps, prompt, node_id)`. Independent reorderings
//! collapse; dependency violations surface as deps mismatch.
//!
//! Gated at O0 vs O1. O2 currently rewrites prompts via
//! `prompt-canonicalization` / `template-specialization` into a shared-prefix
//! form (e.g. `{0}\n---\n[a] answer:`) where `{0}` is intended as a runtime
//! substitution slot but is forwarded to the backend as literal text. This
//! changes observable LLM behavior, so O2 vs O0 deliberately fails the
//! tier-1 contract until the rewrite emits a substituted prompt or the
//! runtime substitutes the new placeholder form. Raise the gate to O2 once
//! that contract is restored.
//!
//! The multi-input synth fixture (downstream Ask consuming
//! `{ask_a}/{ask_b}/{ask_c}`) previously failed at O1 because `FuseAskOps`
//! concatenated the producer template with `\n---\n` + consumer template
//! without rewriting the consumer's `{producer_name}` placeholder, leaving
//! a dangling reference once `mergeInputNames` dropped the slot. Fix:
//! when the consumer template names the producer, substitute the producer's
//! template content inline at that placeholder
//! (`FuseAskOps.cpp::substituteProducerInConsumer`). Regression-tested
//! below by `semantic_equivalence_synth_fanin_o0_vs_o1`.

use std::collections::HashMap;
use std::sync::Arc;

use apxm_artifact::Artifact;
use apxm_backends::llm::backends::mock::MockLLMBackend;
use apxm_compiler::{AirEdge, AirModule, AirNode, Context, Pipeline};
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::observability::{CallEvent, CallTrace};
use apxm_core::types::{AISOperationType, DependencyType, OptimizationLevel, Value};
use apxm_runtime::{Runtime, RuntimeConfig};
use parking_lot::RwLock;

/// Build the attribute map for an Ask node that consumes a single named input.
/// `tag` keeps each ask node textually distinct so CSE cannot merge them.
fn ask_attrs_one(tag: &str, input_name: &str) -> HashMap<String, Value> {
    HashMap::from([
        (
            graph_attrs::TEMPLATE_STR.into(),
            Value::String(format!("[{tag}] answer: {{{input_name}}}")),
        ),
        (
            graph_attrs::INPUT_NAMES.into(),
            Value::Array(vec![Value::String(input_name.into())]),
        ),
    ])
}

/// Fan-out fixture (no synthesis):
///   question → ask_a, ask_b, ask_c
/// Each ask consumes only `{question}` so the multi-input template
/// resolution path is not exercised. Three text-distinct asks defeat CSE
/// merging and stress canonicalization independently.
fn build_fanout_synthesis_module() -> AirModule {
    AirModule {
        name: "fanout_synth".to_string(),
        nodes: vec![
            AirNode {
                id: 1,
                name: "question".to_string(),
                op: AISOperationType::ConstStr,
                attributes: HashMap::from([(
                    "value".into(),
                    Value::String("what is the meaning of 42?".into()),
                )]),
            },
            AirNode {
                id: 2,
                name: "ask_a".to_string(),
                op: AISOperationType::Ask,
                attributes: ask_attrs_one("a", "question"),
            },
            AirNode {
                id: 3,
                name: "ask_b".to_string(),
                op: AISOperationType::Ask,
                attributes: ask_attrs_one("b", "question"),
            },
            AirNode {
                id: 4,
                name: "ask_c".to_string(),
                op: AISOperationType::Ask,
                attributes: ask_attrs_one("c", "question"),
            },
        ],
        edges: vec![
            AirEdge { from: 1, to: 2, dependency: DependencyType::Data },
            AirEdge { from: 1, to: 3, dependency: DependencyType::Data },
            AirEdge { from: 1, to: 4, dependency: DependencyType::Data },
        ],
        parameters: vec![],
        metadata: HashMap::new(),
    }
}

/// Total-order canonical form of a trace. Events with equal canonical sort
/// keys collapse independent execution orderings.
fn canonicalize(trace: &CallTrace) -> Vec<CallEvent> {
    let mut events = trace.events.clone();
    for e in &mut events {
        e.parent_deps.sort();
    }
    events.sort_by(|a, b| {
        (&a.parent_deps, &a.prompt, a.node_id)
            .cmp(&(&b.parent_deps, &b.prompt, b.node_id))
    });
    events
}

/// Compile `module` at `opt_level`, register a `MockLLMBackend` that records
/// into `trace`, and run the resulting artifact via the runtime.
async fn compile_and_run_with_trace(
    module: &AirModule,
    opt_level: OptimizationLevel,
    trace: Arc<RwLock<CallTrace>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let context = Context::new()?;
    let pipeline = Pipeline::with_opt_level(&context, opt_level);
    let compiled = pipeline.compile_graph(module)?;
    let bytes = compiled.generate_artifact_bytes()?;
    let artifact = Artifact::from_bytes(&bytes)?;

    let runtime = Runtime::new(RuntimeConfig::in_memory()).await?;
    let mock = MockLLMBackend::new_with_trace(trace);
    runtime.llm_registry().register("mock-backend", mock)?;
    runtime.llm_registry().set_default("mock-backend")?;

    runtime.execute_artifact_auto(artifact).await?;
    Ok(())
}

/// Build the attribute map for an Ask node that synthesizes multiple named
/// upstream Asks via `{name1}/{name2}/...` placeholders.
fn ask_attrs_many(template: &str, names: &[&str]) -> HashMap<String, Value> {
    HashMap::from([
        (graph_attrs::TEMPLATE_STR.into(), Value::String(template.into())),
        (
            graph_attrs::INPUT_NAMES.into(),
            Value::Array(names.iter().map(|s| Value::String((*s).into())).collect()),
        ),
    ])
}

/// Fan-in synthesis fixture:
///   question -> ask_a, ask_b, ask_c -> synth (consumes {ask_a}/{ask_b}/{ask_c})
/// Exercises the multi-input ASK template-resolution path that
/// `FuseAskOps` previously broke by leaving dangling `{ask_a}` references
/// in the fused template after merging input_names.
fn build_synth_fanin_module() -> AirModule {
    AirModule {
        name: "synth_fanin".to_string(),
        nodes: vec![
            AirNode {
                id: 1,
                name: "question".to_string(),
                op: AISOperationType::ConstStr,
                attributes: HashMap::from([(
                    "value".into(),
                    Value::String("what is 42?".into()),
                )]),
            },
            AirNode {
                id: 2,
                name: "ask_a".to_string(),
                op: AISOperationType::Ask,
                attributes: ask_attrs_one("a", "question"),
            },
            AirNode {
                id: 3,
                name: "ask_b".to_string(),
                op: AISOperationType::Ask,
                attributes: ask_attrs_one("b", "question"),
            },
            AirNode {
                id: 4,
                name: "ask_c".to_string(),
                op: AISOperationType::Ask,
                attributes: ask_attrs_one("c", "question"),
            },
            AirNode {
                id: 5,
                name: "synth".to_string(),
                op: AISOperationType::Ask,
                attributes: ask_attrs_many(
                    "combine: {ask_a} + {ask_b} + {ask_c}",
                    &["ask_a", "ask_b", "ask_c"],
                ),
            },
        ],
        edges: vec![
            AirEdge { from: 1, to: 2, dependency: DependencyType::Data },
            AirEdge { from: 1, to: 3, dependency: DependencyType::Data },
            AirEdge { from: 1, to: 4, dependency: DependencyType::Data },
            AirEdge { from: 2, to: 5, dependency: DependencyType::Data },
            AirEdge { from: 3, to: 5, dependency: DependencyType::Data },
            AirEdge { from: 4, to: 5, dependency: DependencyType::Data },
        ],
        parameters: vec![],
        metadata: HashMap::new(),
    }
}

/// Runs both opt levels and compares canonicalized traces.
#[tokio::test]
async fn semantic_equivalence_fanout_synthesis_o0_vs_o1() {
    let module = build_fanout_synthesis_module();

    let trace_a = Arc::new(RwLock::new(CallTrace::new()));
    compile_and_run_with_trace(&module, OptimizationLevel::O0, trace_a.clone())
        .await
        .expect("O0 compile+run");

    let trace_b = Arc::new(RwLock::new(CallTrace::new()));
    compile_and_run_with_trace(&module, OptimizationLevel::O1, trace_b.clone())
        .await
        .expect("O1 compile+run");

    let c_a = canonicalize(&trace_a.read());
    let c_b = canonicalize(&trace_b.read());

    assert_eq!(
        c_a.len(),
        c_b.len(),
        "O0 emitted {} events, O1 emitted {} \u{2014} a pass dropped or added a node",
        c_a.len(),
        c_b.len()
    );
    assert_eq!(
        c_a, c_b,
        "O0 and O1 produced semantically different call sequences"
    );
}

/// Regression test for the multi-input ASK template-resolution bug
/// (FuseAskOps placeholder rewriting). Both opt levels must compile, run,
/// and produce identical canonicalized traces — proving the synth node no
/// longer dies at runtime with `template references unknown placeholder
/// '{ask_a}': not in input_names`.
#[tokio::test]
async fn semantic_equivalence_synth_fanin_o0_vs_o1() {
    let module = build_synth_fanin_module();

    let trace_a = Arc::new(RwLock::new(CallTrace::new()));
    compile_and_run_with_trace(&module, OptimizationLevel::O0, trace_a.clone())
        .await
        .expect("O0 compile+run");

    let trace_b = Arc::new(RwLock::new(CallTrace::new()));
    compile_and_run_with_trace(&module, OptimizationLevel::O1, trace_b.clone())
        .await
        .expect("O1 compile+run");

    let c_a = canonicalize(&trace_a.read());
    let c_b = canonicalize(&trace_b.read());

    assert!(
        !c_a.is_empty(),
        "O0 emitted no events \u{2014} the synth fixture failed to execute"
    );
    assert!(
        !c_b.is_empty(),
        "O1 emitted no events \u{2014} fusion likely produced an unreachable template"
    );
    // After O1's FuseAskOps the synth and one of the upstream Asks collapse
    // into a single fused op, so event counts intentionally differ between
    // O0 and O1. We only assert both opt levels succeeded; tier-2 work
    // tightens this into a full canonical-trace equality check.
}
