//! Shared-prefix metadata semantics for direct AIR compilation.

use apxm_compiler::{Context, Pipeline};
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::types::{AISOperationType, OptimizationLevel, Value};

const DIRECT_AIR: &str = r#"
module {
  func.func @shared_prefix() -> !ais.token attributes {ais.entry} {
    %source = ais.ask "Source document." : !ais.token
    %left = ais.ask "Instruction: {source}\nQuestion: left" [%source : !ais.token] {input_names = ["source"], model = "gpt-4o"} : !ais.token
    %right = ais.ask "Instruction: {source}\nQuestion: right" [%source : !ais.token] {input_names = ["source"], model = "gpt-4o"} : !ais.token
    %joined = ais.wait_all %left, %right : !ais.token, !ais.token -> !ais.token
    func.return %joined : !ais.token
  }
}
"#;

#[test]
fn direct_air_artifact_uses_the_static_prefix_definition() {
    let context = Context::new().expect("compiler context");
    let pipeline = Pipeline::with_opt_level(&context, OptimizationLevel::O2);
    let module = pipeline.compile(DIRECT_AIR).expect("direct AIR compiles");
    let artifact = module.generate_artifact().expect("artifact emits");
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
        apxm_compiler::token_estimate::count_text_tokens(Some("gpt-4o"), "Instruction: ") as u64;
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
            "direct AIR has no pre-MLIR tokenizer annotation, so it must not nominate a warmup"
        );
    }
}
