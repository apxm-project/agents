//! O0 still has to produce executable artifacts.
//!
//! These tests guard the boundary between required AIR-to-runtime lowering and
//! optional optimization. O0 may skip cleanup, scheduling, priority, DSPy, and
//! graph rewrites, but it must still normalize the graph and materialize the
//! prompt/input_names contract that runtime template rendering depends on.

use std::collections::HashMap;

use apxm_artifact::Artifact;
use apxm_compiler::{AirEdge, AirModule, AirNode, Context, Pipeline};
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::types::{AISOperationType, DependencyType, OptimizationLevel, Value};

fn graph_with_implicit_prompt() -> AirModule {
    AirModule {
        name: "o0_lowering_contract".to_string(),
        nodes: vec![
            AirNode {
                id: 1,
                name: "seed".to_string(),
                op: AISOperationType::ConstStr,
                attributes: HashMap::from([(
                    graph_attrs::VALUE.to_string(),
                    Value::String("hello".to_string()),
                )]),
            },
            AirNode {
                id: 2,
                name: "review".to_string(),
                op: AISOperationType::Ask,
                attributes: HashMap::new(),
            },
        ],
        edges: vec![AirEdge {
            from: 1,
            to: 2,
            dependency: DependencyType::Data,
        }],
        parameters: vec![],
        metadata: HashMap::new(),
    }
}

fn graph_with_nonempty_prompt_and_missing_input_names() -> AirModule {
    AirModule {
        name: "o0_input_names_contract".to_string(),
        nodes: vec![
            AirNode {
                id: 1,
                name: "seed".to_string(),
                op: AISOperationType::ConstStr,
                attributes: HashMap::from([(
                    graph_attrs::VALUE.to_string(),
                    Value::String("hello".to_string()),
                )]),
            },
            AirNode {
                id: 2,
                name: "review".to_string(),
                op: AISOperationType::Ask,
                attributes: HashMap::from([(
                    graph_attrs::TEMPLATE_STR.to_string(),
                    Value::String("Summarize the upstream artifact.".to_string()),
                )]),
            },
        ],
        edges: vec![AirEdge {
            from: 1,
            to: 2,
            dependency: DependencyType::Data,
        }],
        parameters: vec![],
        metadata: HashMap::new(),
    }
}

#[test]
fn o0_materializes_prompt_contract_for_context_only_llm_ops() {
    let context = Context::new().expect("compiler context");
    let pipeline = Pipeline::with_opt_level(&context, OptimizationLevel::O0);

    let module = pipeline
        .compile_graph(&graph_with_implicit_prompt())
        .expect("O0 compile");
    let artifact = Artifact::from_bytes(
        &module
            .generate_artifact_bytes()
            .expect("O0 executable artifact"),
    )
    .expect("artifact parses");
    let dag = artifact.entry_dag().expect("entry dag");
    let ask = dag
        .nodes
        .iter()
        .find(|node| node.op_type == AISOperationType::Ask)
        .expect("ask node");

    assert_eq!(
        ask.attributes.get(graph_attrs::TEMPLATE_STR),
        Some(&Value::String("{ctx0}".to_string()))
    );
    assert_eq!(
        ask.attributes.get(graph_attrs::INPUT_NAMES),
        Some(&Value::Array(vec![Value::String("ctx0".to_string())]))
    );
    assert_eq!(ask.input_tokens.len(), 1);
}

#[test]
fn o0_materializes_input_names_for_nonempty_llm_templates() {
    let context = Context::new().expect("compiler context");
    let pipeline = Pipeline::with_opt_level(&context, OptimizationLevel::O0);

    let module = pipeline
        .compile_graph(&graph_with_nonempty_prompt_and_missing_input_names())
        .expect("O0 compile");
    let artifact = Artifact::from_bytes(
        &module
            .generate_artifact_bytes()
            .expect("O0 executable artifact"),
    )
    .expect("artifact parses");
    let dag = artifact.entry_dag().expect("entry dag");
    let ask = dag
        .nodes
        .iter()
        .find(|node| node.op_type == AISOperationType::Ask)
        .expect("ask node");

    assert_eq!(
        ask.attributes.get(graph_attrs::TEMPLATE_STR),
        Some(&Value::String(
            "Summarize the upstream artifact.".to_string()
        ))
    );
    assert_eq!(
        ask.attributes.get(graph_attrs::INPUT_NAMES),
        Some(&Value::Array(vec![Value::String("ctx0".to_string())]))
    );
    assert_eq!(ask.input_tokens.len(), 1);
}

#[test]
fn normalization_preserves_named_duplicate_context_operands() {
    let context = Context::new().expect("compiler context");
    let pipeline = Pipeline::with_opt_level(&context, OptimizationLevel::O0);
    let air = r#"module {
  func.func @named_duplicate_context() -> !ais.token attributes {ais.entry} {
    %seed = ais.const_str "same value" : !ais.token
    %ask = ais.ask "{left} {right}" [%seed, %seed : !ais.token, !ais.token] {input_names = ["left", "right"]} : !ais.token
    func.return %ask : !ais.token
  }
}
"#;

    let module = pipeline.compile(air).expect("O0 compile");
    let artifact = Artifact::from_bytes(
        &module
            .generate_artifact_bytes()
            .expect("O0 executable artifact"),
    )
    .expect("artifact parses");
    let dag = artifact.entry_dag().expect("entry dag");
    let ask = dag
        .nodes
        .iter()
        .find(|node| node.op_type == AISOperationType::Ask)
        .expect("ask node");

    assert_eq!(ask.input_tokens.len(), 2);
    assert_eq!(
        ask.attributes.get(graph_attrs::INPUT_NAMES),
        Some(&Value::Array(vec![
            Value::String("left".to_string()),
            Value::String("right".to_string())
        ]))
    );
}
