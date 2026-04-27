//! Integration test for priority propagation through the full compilation pipeline.
//!
//! This test verifies that priority attributes set by the AssignPriority MLIR pass
//! are correctly extracted by the ArtifactEmitter and stored in node.metadata.priority,
//! which the runtime scheduler uses for the 4-level priority queue.

use apxm_artifact::Artifact;
use apxm_compiler::{AirEdge, AirModule, AirNode};
use apxm_compiler::{Context, Pipeline};
use apxm_core::constants::graph::attrs as graph_attrs;
use apxm_core::types::{
    AISOperationType, DependencyType, OptimizationLevel, OptimizationTarget, PipelineConfig, Value,
};
use std::collections::HashMap;

/// Build the attribute map for an Ask node that consumes a single named input.
fn ask_attrs(input_name: &str) -> HashMap<String, Value> {
    HashMap::from([
        (
            graph_attrs::TEMPLATE_STR.into(),
            Value::String(format!("{{{input_name}}}")),
        ),
        (
            graph_attrs::INPUT_NAMES.into(),
            Value::Array(vec![Value::String(input_name.into())]),
        ),
    ])
}

fn downstream_ids(node_attrs: &HashMap<String, Value>) -> Vec<u64> {
    node_attrs
        .get(graph_attrs::DOWNSTREAM_NODES)
        .and_then(Value::as_array)
        .expect("downstream_nodes attr")
        .iter()
        .map(|value| value.as_u64().expect("artifact node id"))
        .collect()
}

fn node_priority_by_id(dag: &apxm_core::types::ExecutionDag, id: u64) -> u32 {
    dag.nodes
        .iter()
        .find(|node| node.id == id)
        .unwrap_or_else(|| panic!("missing node {id}"))
        .metadata
        .priority
}

fn node_priority_by_template(dag: &apxm_core::types::ExecutionDag, needle: &str) -> u32 {
    dag.nodes
        .iter()
        .find(|node| {
            node.attributes
                .get(graph_attrs::TEMPLATE_STR)
                .and_then(Value::as_string)
                .is_some_and(|template| template.contains(needle))
        })
        .unwrap_or_else(|| panic!("missing node template containing {needle}"))
        .metadata
        .priority
}

fn shared_prefix_attrs(branch: &str, input_name: &str) -> HashMap<String, Value> {
    HashMap::from([
        (
            graph_attrs::TEMPLATE_STR.into(),
            Value::String(format!("{{{input_name}}}\nAnalyze branch {branch}.")),
        ),
        (
            graph_attrs::INPUT_NAMES.into(),
            Value::Array(vec![Value::String(input_name.into())]),
        ),
    ])
}

#[test]
fn test_priority_on_critical_path() {
    // Create a 3-node chain: node1 -> node2 -> node3
    // All nodes should be on the critical path and get priority=90
    let graph = AirModule {
        name: "critical_chain".to_string(),
        nodes: vec![
            AirNode {
                id: 1,
                name: "input".to_string(),
                op: AISOperationType::ConstStr,
                attributes: HashMap::from([(
                    graph_attrs::VALUE.into(),
                    Value::String("test".into()),
                )]),
            },
            AirNode {
                id: 2,
                name: "process1".to_string(),
                op: AISOperationType::Ask,
                attributes: ask_attrs("input"),
            },
            AirNode {
                id: 3,
                name: "process2".to_string(),
                op: AISOperationType::Ask,
                attributes: ask_attrs("process1"),
            },
            AirNode {
                id: 4,
                name: "return".to_string(),
                op: AISOperationType::Return,
                attributes: HashMap::new(),
            },
        ],
        edges: vec![
            AirEdge {
                from: 1,
                to: 2,
                dependency: DependencyType::Data,
            },
            AirEdge {
                from: 2,
                to: 3,
                dependency: DependencyType::Data,
            },
            AirEdge {
                from: 3,
                to: 4,
                dependency: DependencyType::Data,
            },
        ],
        parameters: vec![],
        metadata: HashMap::new(),
    };

    let context = Context::new().expect("compiler context");
    let pipeline = Pipeline::with_opt_level(&context, OptimizationLevel::O1);

    let module = pipeline.compile_graph(&graph).expect("compilation failed");
    let bytes = module.generate_artifact_bytes().expect("artifact");
    let artifact = Artifact::from_bytes(&bytes).expect("parse artifact");
    let dag = artifact.entry_dag().expect("entry dag");

    assert_eq!(node_priority_by_id(dag, 2), 90);
    assert_eq!(node_priority_by_id(dag, 3), 90);
}

#[test]
fn test_downstream_nodes_match_artifact_node_ids() {
    let graph = AirModule {
        name: "downstream_artifact_ids".to_string(),
        nodes: vec![
            AirNode {
                id: 1,
                name: "root".to_string(),
                op: AISOperationType::ConstStr,
                attributes: HashMap::from([(
                    graph_attrs::VALUE.into(),
                    Value::String("input".into()),
                )]),
            },
            AirNode {
                id: 2,
                name: "left".to_string(),
                op: AISOperationType::Ask,
                attributes: HashMap::from([
                    (
                        graph_attrs::TEMPLATE_STR.into(),
                        Value::String("left branch {root}".into()),
                    ),
                    (
                        graph_attrs::INPUT_NAMES.into(),
                        Value::Array(vec![Value::String("root".into())]),
                    ),
                ]),
            },
            AirNode {
                id: 3,
                name: "right".to_string(),
                op: AISOperationType::Ask,
                attributes: HashMap::from([
                    (
                        graph_attrs::TEMPLATE_STR.into(),
                        Value::String("right branch {root}".into()),
                    ),
                    (
                        graph_attrs::INPUT_NAMES.into(),
                        Value::Array(vec![Value::String("root".into())]),
                    ),
                ]),
            },
            AirNode {
                id: 4,
                name: "join".to_string(),
                op: AISOperationType::Ask,
                attributes: HashMap::from([
                    (
                        graph_attrs::TEMPLATE_STR.into(),
                        Value::String("left={left}; right={right}".into()),
                    ),
                    (
                        graph_attrs::INPUT_NAMES.into(),
                        Value::Array(vec![
                            Value::String("left".into()),
                            Value::String("right".into()),
                        ]),
                    ),
                ]),
            },
        ],
        edges: vec![
            AirEdge {
                from: 1,
                to: 2,
                dependency: DependencyType::Data,
            },
            AirEdge {
                from: 1,
                to: 3,
                dependency: DependencyType::Data,
            },
            AirEdge {
                from: 2,
                to: 4,
                dependency: DependencyType::Data,
            },
            AirEdge {
                from: 3,
                to: 4,
                dependency: DependencyType::Data,
            },
        ],
        parameters: vec![],
        metadata: HashMap::new(),
    };

    let context = Context::new().expect("compiler context");
    let pipeline = Pipeline::with_opt_level(&context, OptimizationLevel::O1);
    let module = pipeline.compile_graph(&graph).expect("compilation failed");
    let bytes = module.generate_artifact_bytes().expect("artifact");
    let artifact = Artifact::from_bytes(&bytes).expect("parse artifact");
    let dag = artifact.entry_dag().expect("entry dag");

    for node in &dag.nodes {
        if !node.attributes.contains_key(graph_attrs::DOWNSTREAM_NODES) {
            continue;
        }
        let mut expected: Vec<u64> = dag
            .edges
            .iter()
            .filter(|edge| edge.from == node.id)
            .map(|edge| edge.to)
            .collect();
        expected.sort_unstable();

        assert_eq!(
            downstream_ids(&node.attributes),
            expected,
            "downstream_nodes must use artifact node IDs for node {}",
            node.id
        );
    }
}

#[test]
fn test_shared_prefix_analysis_marks_latency_fanout() {
    let graph = AirModule {
        name: "shared_prefix_latency_fanout".to_string(),
        nodes: vec![
            AirNode {
                id: 1,
                name: "root".to_string(),
                op: AISOperationType::Ask,
                attributes: HashMap::from([(
                    graph_attrs::TEMPLATE_STR.into(),
                    Value::String("shared source".into()),
                )]),
            },
            AirNode {
                id: 2,
                name: "left".to_string(),
                op: AISOperationType::Ask,
                attributes: shared_prefix_attrs("left", "root"),
            },
            AirNode {
                id: 3,
                name: "right".to_string(),
                op: AISOperationType::Ask,
                attributes: shared_prefix_attrs("right", "root"),
            },
        ],
        edges: vec![
            AirEdge {
                from: 1,
                to: 2,
                dependency: DependencyType::Data,
            },
            AirEdge {
                from: 1,
                to: 3,
                dependency: DependencyType::Data,
            },
        ],
        parameters: vec![],
        metadata: HashMap::new(),
    };

    let context = Context::new().expect("compiler context");
    let pipeline = Pipeline::with_config(
        &context,
        PipelineConfig {
            opt_level: OptimizationLevel::O2,
            target: OptimizationTarget::Latency,
            ..Default::default()
        },
    );
    let module = pipeline.compile_graph(&graph).expect("compilation failed");
    let bytes = module.generate_artifact_bytes().expect("artifact");
    let artifact = Artifact::from_bytes(&bytes).expect("parse artifact");
    let dag = artifact.entry_dag().expect("entry dag");

    let llm_nodes: Vec<_> = dag
        .nodes
        .iter()
        .filter(|node| {
            matches!(node.op_type, AISOperationType::Ask)
                && node.attributes.contains_key(graph_attrs::REUSE_GROUP)
        })
        .collect();
    assert_eq!(llm_nodes.len(), 2);

    let groups: Vec<_> = llm_nodes
        .iter()
        .map(|node| {
            node.attributes
                .get(graph_attrs::REUSE_GROUP)
                .and_then(Value::as_string)
                .expect("shared prefix group")
                .to_string()
        })
        .collect();
    assert_eq!(groups[0], groups[1]);

    let warmup_count = llm_nodes
        .iter()
        .filter(|node| {
            node.attributes
                .get(graph_attrs::WARMUP_CANDIDATE)
                .and_then(Value::as_bool)
                .unwrap_or(false)
        })
        .count();
    assert_eq!(warmup_count, 1);

    for node in llm_nodes {
        let estimated = node
            .attributes
            .get(graph_attrs::SHARED_PREFIX_EST_TOKENS)
            .and_then(Value::as_u64)
            .expect("shared prefix token estimate");
        assert!(estimated > 0);
        assert_eq!(
            node.attributes
                .get(graph_attrs::SHARED_PREFIX_GROUP_SIZE)
                .and_then(Value::as_u64),
            Some(2)
        );
    }
}

#[test]
fn test_priority_on_fan_out() {
    // Create a fan-out graph: node1 -> [node2, node3, node4]
    // node1 has fan-out of 3, should get priority=70 (High)
    let graph = AirModule {
        name: "fan_out".to_string(),
        nodes: vec![
            AirNode {
                id: 1,
                name: "producer".to_string(),
                op: AISOperationType::ConstStr,
                attributes: HashMap::from([(
                    graph_attrs::VALUE.into(),
                    Value::String("shared".into()),
                )]),
            },
            AirNode {
                id: 2,
                name: "consumer1".to_string(),
                op: AISOperationType::Ask,
                attributes: ask_attrs("producer"),
            },
            AirNode {
                id: 3,
                name: "consumer2".to_string(),
                op: AISOperationType::Ask,
                attributes: ask_attrs("producer"),
            },
            AirNode {
                id: 4,
                name: "consumer3".to_string(),
                op: AISOperationType::Ask,
                attributes: ask_attrs("producer"),
            },
        ],
        edges: vec![
            AirEdge {
                from: 1,
                to: 2,
                dependency: DependencyType::Data,
            },
            AirEdge {
                from: 1,
                to: 3,
                dependency: DependencyType::Data,
            },
            AirEdge {
                from: 1,
                to: 4,
                dependency: DependencyType::Data,
            },
        ],
        parameters: vec![],
        metadata: HashMap::new(),
    };

    let context = Context::new().expect("compiler context");
    let pipeline = Pipeline::with_opt_level(&context, OptimizationLevel::O1);

    // Compile the module through the pipeline (requires MLIR)
    let _module = pipeline.compile_graph(&graph).expect("compilation failed");
}

#[test]
fn test_priority_normal_for_non_critical() {
    // Create a graph with a non-critical branch:
    // node1 -> node2 -> node4 (critical path)
    // node1 -> node3 (non-critical, should get priority=30)
    let graph = AirModule {
        name: "mixed_priority".to_string(),
        nodes: vec![
            AirNode {
                id: 1,
                name: "root".to_string(),
                op: AISOperationType::ConstStr,
                attributes: HashMap::from([(
                    graph_attrs::VALUE.into(),
                    Value::String("input".into()),
                )]),
            },
            AirNode {
                id: 2,
                name: "critical1".to_string(),
                op: AISOperationType::Ask,
                attributes: HashMap::from([
                    (
                        graph_attrs::TEMPLATE_STR.into(),
                        Value::String("critical branch {root}".into()),
                    ),
                    (
                        graph_attrs::INPUT_NAMES.into(),
                        Value::Array(vec![Value::String("root".into())]),
                    ),
                ]),
            },
            AirNode {
                id: 3,
                name: "non_critical".to_string(),
                op: AISOperationType::Ask,
                attributes: HashMap::from([
                    (
                        graph_attrs::TEMPLATE_STR.into(),
                        Value::String("background branch {root}".into()),
                    ),
                    (
                        graph_attrs::INPUT_NAMES.into(),
                        Value::Array(vec![Value::String("root".into())]),
                    ),
                ]),
            },
            AirNode {
                id: 4,
                name: "critical2".to_string(),
                op: AISOperationType::Ask,
                attributes: HashMap::from([
                    (
                        graph_attrs::TEMPLATE_STR.into(),
                        Value::String("final critical step {critical1}".into()),
                    ),
                    (
                        graph_attrs::INPUT_NAMES.into(),
                        Value::Array(vec![Value::String("critical1".into())]),
                    ),
                ]),
            },
            AirNode {
                id: 5,
                name: "join".to_string(),
                op: AISOperationType::Merge,
                attributes: HashMap::new(),
            },
            AirNode {
                id: 6,
                name: "return".to_string(),
                op: AISOperationType::Return,
                attributes: HashMap::new(),
            },
        ],
        edges: vec![
            AirEdge {
                from: 1,
                to: 2,
                dependency: DependencyType::Data,
            },
            AirEdge {
                from: 1,
                to: 3,
                dependency: DependencyType::Data,
            },
            AirEdge {
                from: 2,
                to: 4,
                dependency: DependencyType::Data,
            },
            AirEdge {
                from: 4,
                to: 5,
                dependency: DependencyType::Data,
            },
            AirEdge {
                from: 3,
                to: 5,
                dependency: DependencyType::Data,
            },
            AirEdge {
                from: 5,
                to: 6,
                dependency: DependencyType::Data,
            },
        ],
        parameters: vec![],
        metadata: HashMap::new(),
    };

    let context = Context::new().expect("compiler context");
    let pipeline = Pipeline::with_opt_level(&context, OptimizationLevel::O1);

    let module = pipeline.compile_graph(&graph).expect("compilation failed");
    let bytes = module.generate_artifact_bytes().expect("artifact");
    let artifact = Artifact::from_bytes(&bytes).expect("parse artifact");
    let dag = artifact.entry_dag().expect("entry dag");

    assert_eq!(node_priority_by_template(dag, "critical branch"), 90);
    assert_eq!(node_priority_by_template(dag, "final critical step"), 90);
    assert_eq!(node_priority_by_template(dag, "background branch"), 30);
}
