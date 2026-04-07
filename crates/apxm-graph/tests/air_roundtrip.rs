//! Round-trip tests for .air format parsing and emission.

use apxm_core::constants::graph::attrs;
use apxm_core::types::{AISOperationType, DependencyType, Value};
use apxm_graph::{ApxmGraph, GraphEdge, GraphNode, Parameter};
use std::collections::HashMap;

#[test]
fn air_roundtrip_simple_graph() {
    // Create a simple graph
    let graph = ApxmGraph {
        name: "roundtrip_test".to_string(),
        nodes: vec![
            GraphNode {
                id: 1,
                name: "input".to_string(),
                op: AISOperationType::ConstStr,
                attributes: HashMap::from([("value".into(), Value::String("test input".into()))]),
            },
            GraphNode {
                id: 2,
                name: "process".to_string(),
                op: AISOperationType::Ask,
                attributes: HashMap::from([(
                    attrs::TEMPLATE_STR.into(),
                    Value::String("Process: {0}".into()),
                )]),
            },
        ],
        edges: vec![GraphEdge {
            from: 1,
            to: 2,
            dependency: DependencyType::Data,
        }],
        parameters: vec![],
        metadata: HashMap::new(),
    };

    // Convert to .air
    let air_text = emit_air(&graph);

    // Parse back
    let parsed = ApxmGraph::from_air(&air_text).expect("parse failed");

    // Verify structure
    assert_eq!(parsed.name, graph.name);
    assert_eq!(parsed.nodes.len(), graph.nodes.len());
    assert_eq!(parsed.edges.len(), graph.edges.len());

    // Verify nodes
    assert_eq!(parsed.nodes[0].name, "input");
    assert_eq!(parsed.nodes[0].op, AISOperationType::ConstStr);
    assert_eq!(parsed.nodes[1].name, "process");
    assert_eq!(parsed.nodes[1].op, AISOperationType::Ask);

    // Verify edges
    assert_eq!(parsed.edges[0].from, 1);
    assert_eq!(parsed.edges[0].to, 2);
    assert_eq!(parsed.edges[0].dependency, DependencyType::Data);
}

#[test]
fn air_roundtrip_with_parameters() {
    let graph = ApxmGraph {
        name: "param_test".to_string(),
        nodes: vec![GraphNode {
            id: 1,
            name: "task".to_string(),
            op: AISOperationType::ConstStr,
            attributes: HashMap::from([("value".into(), Value::String("{0}".into()))]),
        }],
        edges: vec![],
        parameters: vec![
            Parameter {
                name: "input".to_string(),
                type_name: "str".to_string(),
            },
            Parameter {
                name: "count".to_string(),
                type_name: "int".to_string(),
            },
        ],
        metadata: HashMap::new(),
    };

    let air_text = emit_air(&graph);
    let parsed = ApxmGraph::from_air(&air_text).expect("parse failed");

    assert_eq!(parsed.parameters.len(), 2);
    assert_eq!(parsed.parameters[0].name, "input");
    assert_eq!(parsed.parameters[0].type_name, "str");
    assert_eq!(parsed.parameters[1].name, "count");
    assert_eq!(parsed.parameters[1].type_name, "int");
}

#[test]
fn air_roundtrip_with_metadata() {
    let mut metadata = HashMap::new();
    metadata.insert("is_entry".to_string(), Value::Bool(true));
    metadata.insert(
        "description".to_string(),
        Value::String("Test workflow".to_string()),
    );

    let graph = ApxmGraph {
        name: "meta_test".to_string(),
        nodes: vec![GraphNode {
            id: 1,
            name: "noop".to_string(),
            op: AISOperationType::Nop,
            attributes: HashMap::new(),
        }],
        edges: vec![],
        parameters: vec![],
        metadata,
    };

    let air_text = emit_air(&graph);
    let parsed = ApxmGraph::from_air(&air_text).expect("parse failed");

    assert_eq!(
        parsed.metadata.get("is_entry"),
        Some(&Value::Bool(true))
    );
    assert_eq!(
        parsed.metadata.get("description"),
        Some(&Value::String("Test workflow".to_string()))
    );
}

#[test]
fn air_roundtrip_complex_workflow() {
    let graph = ApxmGraph {
        name: "complex".to_string(),
        nodes: vec![
            GraphNode {
                id: 1,
                name: "spawn".to_string(),
                op: AISOperationType::SpawnAgent,
                attributes: HashMap::from([
                    ("agent_name".into(), Value::String("worker".into())),
                    ("profile".into(), Value::String("claude".into())),
                ]),
            },
            GraphNode {
                id: 2,
                name: "task".to_string(),
                op: AISOperationType::ConstStr,
                attributes: HashMap::from([("value".into(), Value::String("Do work".into()))]),
            },
            GraphNode {
                id: 3,
                name: "send".to_string(),
                op: AISOperationType::Communicate,
                attributes: HashMap::from([
                    ("recipient".into(), Value::String("worker".into())),
                    ("protocol".into(), Value::String("acp".into())),
                ]),
            },
        ],
        edges: vec![
            GraphEdge {
                from: 1,
                to: 3,
                dependency: DependencyType::Control,
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

    let air_text = emit_air(&graph);
    let parsed = ApxmGraph::from_air(&air_text).expect("parse failed");

    // Verify all nodes
    assert_eq!(parsed.nodes.len(), 3);
    assert_eq!(parsed.nodes[0].op, AISOperationType::SpawnAgent);
    assert_eq!(parsed.nodes[1].op, AISOperationType::ConstStr);
    assert_eq!(parsed.nodes[2].op, AISOperationType::Communicate);

    // Verify all edges and dependency types
    assert_eq!(parsed.edges.len(), 2);
    assert_eq!(parsed.edges[0].dependency, DependencyType::Control);
    assert_eq!(parsed.edges[1].dependency, DependencyType::Data);
}

#[test]
fn air_roundtrip_special_characters() {
    let graph = ApxmGraph {
        name: "escape_test".to_string(),
        nodes: vec![GraphNode {
            id: 1,
            name: "multiline".to_string(),
            op: AISOperationType::ConstStr,
            attributes: HashMap::from([(
                "value".into(),
                Value::String("Line 1\nLine 2\tTab\tTab".into()),
            )]),
        }],
        edges: vec![],
        parameters: vec![],
        metadata: HashMap::new(),
    };

    let air_text = emit_air(&graph);
    let parsed = ApxmGraph::from_air(&air_text).expect("parse failed");

    let val = parsed.nodes[0].attributes.get("value").unwrap();
    if let Value::String(s) = val {
        assert!(s.contains('\n'));
        assert!(s.contains('\t'));
    } else {
        panic!("Expected string value");
    }
}

/// Helper to emit .air format (uses the driver's emit_air)
fn emit_air(graph: &ApxmGraph) -> String {
    let mut out = String::new();
    out.push_str("; Agent IR (.air) — canonical intermediate representation\n");
    out.push_str(&format!("; graph: {}\n", graph.name));
    for (k, v) in &graph.metadata {
        // For metadata, format values without quotes (simple string output)
        let val_str = match v {
            Value::String(s) => s.clone(),
            Value::Bool(b) => b.to_string(),
            Value::Number(n) => n.to_string(),
            _ => format_value(v),
        };
        out.push_str(&format!("; {}: {}\n", k, val_str));
    }
    out.push('\n');
    if !graph.parameters.is_empty() {
        out.push_str("; params:\n");
        for p in &graph.parameters {
            out.push_str(&format!(";   %{}: {}\n", p.name, p.type_name));
        }
        out.push('\n');
    }
    for node in &graph.nodes {
        let op = node.op.to_string().to_lowercase();
        let mut attrs = vec![];
        for (k, v) in &node.attributes {
            if !k.starts_with('_') {
                attrs.push(format!("{} = {}", k, format_value(v)));
            }
        }
        let attr_str = if attrs.is_empty() {
            String::new()
        } else {
            format!(" {{{}}}", attrs.join(", "))
        };
        out.push_str(&format!("  %{} = ais.{}{}\n", node.name, op, attr_str));
    }
    if !graph.edges.is_empty() {
        out.push_str("\n  ; edges:\n");
        for edge in &graph.edges {
            let from = graph
                .nodes
                .iter()
                .find(|n| n.id == edge.from)
                .map(|n| n.name.as_str())
                .unwrap_or("?");
            let to = graph
                .nodes
                .iter()
                .find(|n| n.id == edge.to)
                .map(|n| n.name.as_str())
                .unwrap_or("?");
            out.push_str(&format!(
                "  ; %{} -> %{} ({:?})\n",
                from, to, edge.dependency
            ));
        }
    }
    out
}

fn format_value(v: &Value) -> String {
    match v {
        Value::Null => "null".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => {
            // Escape special characters
            let escaped = s
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('\n', "\\n")
                .replace('\t', "\\t");
            format!("\"{}\"", escaped)
        }
        Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(format_value).collect();
            format!("[{}]", items.join(", "))
        }
        Value::Object(_) => "{}".to_string(), // Simplified
        Value::Token(id) => format!("{{{{token_{}}}}}", id),
    }
}
