//! Shared-prefix metadata semantics for direct AIR compilation.

use apxm_compiler::{
    BackendCapabilityEvidence, CompilerAnalysisInputs, ConfiguredBackendEvidence, Context,
    Pipeline, TokenizerEvidence,
};
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::types::{AISOperationType, OptimizationLevel, Value};

const DIRECT_AIR: &str = r#"
module {
  func.func @shared_prefix() -> !ais.token attributes {ais.entry} {
    %source = ais.ask "Source document." : !ais.token
    %left = ais.ask "Instruction: {source}\nQuestion: left" [%source : !ais.token] {input_names = ["source"], input_roles = ["user"], model = "gpt-4o"} : !ais.token
    %right = ais.ask "Instruction: {source}\nQuestion: right" [%source : !ais.token] {input_names = ["source"], input_roles = ["user"], model = "gpt-4o"} : !ais.token
    %joined = ais.wait_all %left, %right : !ais.token, !ais.token -> !ais.token
    func.return %joined : !ais.token
  }
}
"#;

#[test]
fn direct_air_artifact_requires_configured_tokenizer_evidence() {
    let context = Context::new().expect("compiler context");
    let pipeline = Pipeline::with_opt_level(&context, OptimizationLevel::O2);
    let module = pipeline.compile(DIRECT_AIR).expect("direct AIR compiles");
    let unavailable_artifact = module.generate_artifact().expect("artifact emits");
    let unavailable_dag = unavailable_artifact.entry_dag().expect("entry DAG");

    for node in unavailable_dag.nodes.iter().filter(|node| {
        node.op_type == AISOperationType::Ask
            && node.get_attribute(graph_attrs::REUSE_GROUP).is_some()
    }) {
        assert!(
            node.get_attribute(graph_attrs::SHARED_PREFIX_EST_TOKENS)
                .is_none(),
            "model spelling alone cannot create a shared-prefix estimate"
        );
    }

    let tokenizer = TokenizerEvidence::O200kBase;
    let inputs = CompilerAnalysisInputs {
        configured_backends: vec![ConfiguredBackendEvidence {
            backend: "configured-backend".to_string(),
            model: "gpt-4o".to_string(),
            aliases: Default::default(),
            available: true,
            capabilities: BackendCapabilityEvidence {
                tokenizer,
                ..Default::default()
            },
        }],
        ..Default::default()
    };
    let artifact = module
        .generate_artifact_with_analysis_inputs(&inputs)
        .expect("artifact emits with configured tokenizer evidence");
    let dag = artifact.entry_dag().expect("entry DAG");

    let shared: Vec<_> = dag
        .nodes
        .iter()
        .filter(|node| {
            node.op_type == AISOperationType::Ask
                && node.get_attribute(graph_attrs::REUSE_GROUP).is_some()
        })
        .collect();
    assert_eq!(shared.len(), 2);

    let expected =
        apxm_compiler::token_estimate::count_text_tokens_with_evidence(tokenizer, "Instruction: ")
            .expect("configured tokenizer evidence") as u64;
    let group = shared[0]
        .get_attribute(graph_attrs::REUSE_GROUP)
        .and_then(Value::as_str)
        .expect("reuse group");
    for node in shared {
        assert_eq!(
            node.get_attribute(graph_attrs::REUSE_GROUP)
                .and_then(Value::as_str),
            Some(group)
        );
        assert_eq!(
            node.get_attribute(graph_attrs::SHARED_PREFIX_EST_TOKENS)
                .and_then(Value::as_u64),
            Some(expected)
        );
        assert!(
            node.get_attribute(graph_attrs::WARMUP_CANDIDATE).is_none(),
            "artifact refinement does not nominate a pre-dispatch warmup candidate"
        );
    }
}
