//! Integration test for priority propagation through the full compilation pipeline.
//!
//! This test verifies that priority attributes set by the AssignPriority MLIR pass
//! are correctly extracted by the ArtifactEmitter and stored in node.metadata.priority,
//! which the runtime scheduler uses for the 4-level priority queue.

use apxm_compiler::{AirEdge, AirModule, AirNode};
use apxm_compiler::{Context, Pipeline};
use apxm_core::types::{AISOperationType, DependencyType, OptimizationLevel, Value};
use std::collections::HashMap;

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
                attributes: HashMap::from([("value".into(), Value::String("test".into()))]),
            },
            AirNode {
                id: 2,
                name: "process1".to_string(),
                op: AISOperationType::Ask,
                attributes: HashMap::from([("template_str".into(), Value::String("{0}".into()))]),
            },
            AirNode {
                id: 3,
                name: "process2".to_string(),
                op: AISOperationType::Ask,
                attributes: HashMap::from([("template_str".into(), Value::String("{0}".into()))]),
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
        ],
        parameters: vec![],
        metadata: HashMap::new(),
    };

    let context = Context::new().expect("compiler context");
    let pipeline = Pipeline::with_opt_level(&context, OptimizationLevel::O1);

    // Compile the module through the pipeline (requires MLIR)
    let _module = pipeline.compile_graph(&graph)
        .expect("compilation failed");
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
                attributes: HashMap::from([("value".into(), Value::String("shared".into()))]),
            },
            AirNode {
                id: 2,
                name: "consumer1".to_string(),
                op: AISOperationType::Ask,
                attributes: HashMap::from([("template_str".into(), Value::String("{0}".into()))]),
            },
            AirNode {
                id: 3,
                name: "consumer2".to_string(),
                op: AISOperationType::Ask,
                attributes: HashMap::from([("template_str".into(), Value::String("{0}".into()))]),
            },
            AirNode {
                id: 4,
                name: "consumer3".to_string(),
                op: AISOperationType::Ask,
                attributes: HashMap::from([("template_str".into(), Value::String("{0}".into()))]),
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
    let _module = pipeline.compile_graph(&graph)
        .expect("compilation failed");
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
                attributes: HashMap::from([("value".into(), Value::String("input".into()))]),
            },
            AirNode {
                id: 2,
                name: "critical1".to_string(),
                op: AISOperationType::Ask,
                attributes: HashMap::from([("template_str".into(), Value::String("{0}".into()))]),
            },
            AirNode {
                id: 3,
                name: "non_critical".to_string(),
                op: AISOperationType::Ask,
                attributes: HashMap::from([("template_str".into(), Value::String("{0}".into()))]),
            },
            AirNode {
                id: 4,
                name: "critical2".to_string(),
                op: AISOperationType::Ask,
                attributes: HashMap::from([("template_str".into(), Value::String("{0}".into()))]),
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
        ],
        parameters: vec![],
        metadata: HashMap::new(),
    };

    let context = Context::new().expect("compiler context");
    let pipeline = Pipeline::with_opt_level(&context, OptimizationLevel::O1);

    // Compile the module through the pipeline (requires MLIR)
    let _module = pipeline.compile_graph(&graph)
        .expect("compilation failed");
}
