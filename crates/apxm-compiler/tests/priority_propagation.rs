//! Integration test for priority propagation through the full compilation pipeline.
//!
//! This test verifies that priority attributes set by the AssignPriority MLIR pass
//! are correctly extracted by the ArtifactEmitter and stored in node.metadata.priority,
//! which the runtime scheduler uses for the 4-level priority queue.

use apxm_artifact::Artifact;
use apxm_compiler::{compile_graph_to_artifact, CompilerConfig};
use apxm_core::types::{AISOperationType, DependencyType, OptimizationLevel, Value};
use apxm_graph::{ApxmGraph, GraphEdge, GraphNode};
use std::collections::HashMap;

#[test]
fn test_priority_on_critical_path() {
    // Create a 3-node chain: node1 -> node2 -> node3
    // All nodes should be on the critical path and get priority=90
    let graph = ApxmGraph {
        name: "critical_chain".to_string(),
        nodes: vec![
            GraphNode {
                id: 1,
                name: "input".to_string(),
                op: AISOperationType::ConstStr,
                attributes: HashMap::from([("value".into(), Value::String("test".into()))]),
            },
            GraphNode {
                id: 2,
                name: "process1".to_string(),
                op: AISOperationType::Ask,
                attributes: HashMap::from([("template_str".into(), Value::String("{0}".into()))]),
            },
            GraphNode {
                id: 3,
                name: "process2".to_string(),
                op: AISOperationType::Ask,
                attributes: HashMap::from([("template_str".into(), Value::String("{0}".into()))]),
            },
        ],
        edges: vec![
            GraphEdge {
                from: 1,
                to: 2,
                dependency: DependencyType::Data,
            },
            GraphEdge {
                from: 2,
                to: 3,
                dependency: DependencyType::Data,
            },
        ],
        parameters: vec![],
        metadata: HashMap::new(),
    };

    let config = CompilerConfig {
        optimization_level: OptimizationLevel::O1,
        ..Default::default()
    };

    let artifact = compile_graph_to_artifact(&graph, &config)
        .expect("compilation failed");

    let artifact = Artifact::from_bytes(&artifact).expect("deserialization failed");

    // Verify the artifact has the expected number of nodes
    assert_eq!(artifact.dags.len(), 1, "Expected single DAG");
    let dag = &artifact.dags[0];
    assert_eq!(dag.nodes.len(), 3, "Expected 3 nodes in DAG");

    // All nodes on the critical path should have priority=90 (Critical)
    for node in &dag.nodes {
        assert_eq!(
            node.metadata.priority, 90,
            "Node {} should have critical priority (90), got {}",
            node.id, node.metadata.priority
        );
    }
}

#[test]
fn test_priority_on_fan_out() {
    // Create a fan-out graph: node1 -> [node2, node3, node4]
    // node1 has fan-out of 3, should get priority=70 (High)
    let graph = ApxmGraph {
        name: "fan_out".to_string(),
        nodes: vec![
            GraphNode {
                id: 1,
                name: "producer".to_string(),
                op: AISOperationType::ConstStr,
                attributes: HashMap::from([("value".into(), Value::String("shared".into()))]),
            },
            GraphNode {
                id: 2,
                name: "consumer1".to_string(),
                op: AISOperationType::Ask,
                attributes: HashMap::from([("template_str".into(), Value::String("{0}".into()))]),
            },
            GraphNode {
                id: 3,
                name: "consumer2".to_string(),
                op: AISOperationType::Ask,
                attributes: HashMap::from([("template_str".into(), Value::String("{0}".into()))]),
            },
            GraphNode {
                id: 4,
                name: "consumer3".to_string(),
                op: AISOperationType::Ask,
                attributes: HashMap::from([("template_str".into(), Value::String("{0}".into()))]),
            },
        ],
        edges: vec![
            GraphEdge {
                from: 1,
                to: 2,
                dependency: DependencyType::Data,
            },
            GraphEdge {
                from: 1,
                to: 3,
                dependency: DependencyType::Data,
            },
            GraphEdge {
                from: 1,
                to: 4,
                dependency: DependencyType::Data,
            },
        ],
        parameters: vec![],
        metadata: HashMap::new(),
    };

    let config = CompilerConfig {
        optimization_level: OptimizationLevel::O1,
        ..Default::default()
    };

    let artifact = compile_graph_to_artifact(&graph, &config)
        .expect("compilation failed");

    let artifact = Artifact::from_bytes(&artifact).expect("deserialization failed");

    assert_eq!(artifact.dags.len(), 1);
    let dag = &artifact.dags[0];

    // Find the producer node (id=1)
    let producer = dag.nodes.iter().find(|n| n.id == 1).expect("producer not found");

    // Producer with fan-out >= 3 should have priority=70 (High)
    // Note: The AssignPriority pass assigns 70 for fan-out >= 3
    // But since it's also on critical path (path length near max), it might get 90
    // Let's verify it has at least High priority (>= 70)
    assert!(
        producer.metadata.priority >= 70,
        "Producer with high fan-out should have priority >= 70, got {}",
        producer.metadata.priority
    );
}

#[test]
fn test_priority_normal_for_non_critical() {
    // Create a graph with a non-critical branch:
    // node1 -> node2 -> node4 (critical path)
    // node1 -> node3 (non-critical, should get priority=30)
    let graph = ApxmGraph {
        name: "mixed_priority".to_string(),
        nodes: vec![
            GraphNode {
                id: 1,
                name: "root".to_string(),
                op: AISOperationType::ConstStr,
                attributes: HashMap::from([("value".into(), Value::String("input".into()))]),
            },
            GraphNode {
                id: 2,
                name: "critical1".to_string(),
                op: AISOperationType::Ask,
                attributes: HashMap::from([("template_str".into(), Value::String("{0}".into()))]),
            },
            GraphNode {
                id: 3,
                name: "non_critical".to_string(),
                op: AISOperationType::Ask,
                attributes: HashMap::from([("template_str".into(), Value::String("{0}".into()))]),
            },
            GraphNode {
                id: 4,
                name: "critical2".to_string(),
                op: AISOperationType::Ask,
                attributes: HashMap::from([("template_str".into(), Value::String("{0}".into()))]),
            },
        ],
        edges: vec![
            GraphEdge {
                from: 1,
                to: 2,
                dependency: DependencyType::Data,
            },
            GraphEdge {
                from: 1,
                to: 3,
                dependency: DependencyType::Data,
            },
            GraphEdge {
                from: 2,
                to: 4,
                dependency: DependencyType::Data,
            },
        ],
        parameters: vec![],
        metadata: HashMap::new(),
    };

    let config = CompilerConfig {
        optimization_level: OptimizationLevel::O1,
        ..Default::default()
    };

    let artifact = compile_graph_to_artifact(&graph, &config)
        .expect("compilation failed");

    let artifact = Artifact::from_bytes(&artifact).expect("deserialization failed");

    assert_eq!(artifact.dags.len(), 1);
    let dag = &artifact.dags[0];

    // Find the non-critical node (id=3)
    let non_critical = dag.nodes.iter().find(|n| n.id == 3).expect("non_critical not found");

    // Non-critical node with low fan-out should have priority=30 (Normal)
    assert_eq!(
        non_critical.metadata.priority, 30,
        "Non-critical node should have normal priority (30), got {}",
        non_critical.metadata.priority
    );
}
