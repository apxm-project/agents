use std::collections::HashMap;

use crate::api::module::invalid_input_error;
use apxm_core::error::compiler::CompilerError;
use apxm_core::types::execution::{DagMetadata, ExecutionDag, FlowParameter};
use apxm_core::types::values::{Number, Value};
use apxm_core::types::{AISOperationType, DependencyType, Edge, Node, NodeMetadata};

const WIRE_VERSION: u32 = 3;

/// Parse multiple DAGs from wire format v3 (multi-DAG support)
pub fn parse_wire_dags(bytes: &[u8]) -> Result<Vec<ExecutionDag>, CompilerError> {
    let mut reader = BinaryReader::new(bytes);
    let version = reader.read_u32()?;

    if version != WIRE_VERSION {
        return Err(invalid_input_error(format!(
            "Unsupported artifact version {version}, expected {WIRE_VERSION}"
        )));
    }

    let num_dags = reader.read_u64()? as usize;
    let mut dags = Vec::with_capacity(num_dags);

    for _ in 0..num_dags {
        dags.push(parse_single_dag(&mut reader)?);
    }

    Ok(dags)
}

/// Parse a single DAG from the reader (shared logic)
fn parse_single_dag(reader: &mut BinaryReader) -> Result<ExecutionDag, CompilerError> {
    let module_name = reader.read_string()?;
    let is_entry = reader.read_bool()?;

    // Read parameter metadata
    let param_count = reader.read_u64()? as usize;
    let mut parameters = Vec::with_capacity(param_count);
    for _ in 0..param_count {
        let name = reader.read_string()?;
        let type_name = reader.read_string()?;
        parameters.push(FlowParameter { name, type_name });
    }

    let node_count = reader.read_u64()? as usize;
    let mut nodes = Vec::with_capacity(node_count);
    for _ in 0..node_count {
        nodes.push(read_node(reader)?);
    }

    let edge_count = reader.read_u64()? as usize;
    let mut edges = Vec::with_capacity(edge_count);
    for _ in 0..edge_count {
        let from = reader.read_u64()?;
        let to = reader.read_u64()?;
        let token_id = reader.read_u64()?;
        let dependency = match reader.read_u8()? {
            0 => DependencyType::Data,
            1 => DependencyType::Effect,
            2 => DependencyType::Control,
            other => {
                return Err(invalid_input_error(format!(
                    "Unknown dependency kind {other} in artifact"
                )));
            }
        };
        edges.push(Edge {
            from,
            to,
            token_id,
            dependency_type: dependency,
        });
    }

    let entry_count = reader.read_u64()? as usize;
    let mut entry_nodes = Vec::with_capacity(entry_count);
    for _ in 0..entry_count {
        entry_nodes.push(reader.read_u64()?);
    }

    let exit_count = reader.read_u64()? as usize;
    let mut exit_nodes = Vec::with_capacity(exit_count);
    for _ in 0..exit_count {
        exit_nodes.push(reader.read_u64()?);
    }

    let metadata = DagMetadata {
        name: if module_name.is_empty() {
            None
        } else {
            Some(module_name)
        },
        is_entry,
        parameters,
    };

    Ok(ExecutionDag {
        nodes,
        edges,
        entry_nodes,
        exit_nodes,
        metadata,
    })
}

/// Parse wire format and return the explicit @entry DAG.
pub fn parse_wire_dag(bytes: &[u8]) -> Result<ExecutionDag, CompilerError> {
    let dags = parse_wire_dags(bytes)?;

    dags.into_iter()
        .find(|d| d.metadata.is_entry)
        .ok_or_else(|| invalid_input_error("No entry DAG found in artifact"))
}

fn read_node(reader: &mut BinaryReader) -> Result<Node, CompilerError> {
    let id = reader.read_u64()?;
    let op_index = reader.read_u32()?;
    let op_type = AISOperationType::from_wire_index(op_index)
        .ok_or_else(|| invalid_input_error(format!("Unknown operation kind index {op_index}")))?;

    let attr_count = reader.read_u64()? as usize;
    let mut attributes = HashMap::with_capacity(attr_count);
    for _ in 0..attr_count {
        let key = reader.read_string()?;
        let value = read_value(reader)?;
        attributes.insert(key, value);
    }

    let input_count = reader.read_u64()? as usize;
    let mut input_tokens = Vec::with_capacity(input_count);
    for _ in 0..input_count {
        input_tokens.push(reader.read_u64()?);
    }

    let output_count = reader.read_u64()? as usize;
    let mut output_tokens = Vec::with_capacity(output_count);
    for _ in 0..output_count {
        output_tokens.push(reader.read_u64()?);
    }

    let priority = reader.read_u32()?;
    let has_latency = reader.read_bool()?;
    let estimated_latency = if has_latency {
        Some(reader.read_u64()?)
    } else {
        None
    };

    Ok(Node {
        id,
        op_type,
        attributes,
        input_tokens,
        output_tokens,
        metadata: NodeMetadata {
            name: None,
            priority,
            estimated_latency,
            task_source_id: None,
        },
    })
}

fn read_value(reader: &mut BinaryReader) -> Result<Value, CompilerError> {
    match reader.read_u8()? {
        0 => Ok(Value::Null),
        1 => Ok(Value::Bool(reader.read_bool()?)),
        2 => Ok(Value::Number(Number::Integer(reader.read_i64()?))),
        3 => Ok(Value::Number(Number::Float(reader.read_f64()?))),
        4 => Ok(Value::String(reader.read_string()?)),
        5 => {
            let len = reader.read_u64()? as usize;
            let mut values = Vec::with_capacity(len);
            for _ in 0..len {
                values.push(read_value(reader)?);
            }
            Ok(Value::Array(values))
        }
        6 => {
            let len = reader.read_u64()? as usize;
            let mut entries = HashMap::with_capacity(len);
            for _ in 0..len {
                let key = reader.read_string()?;
                let value = read_value(reader)?;
                entries.insert(key, value);
            }
            Ok(Value::Object(entries))
        }
        7 => Ok(Value::Token(reader.read_u64()?)),
        other => Err(invalid_input_error(format!(
            "Unknown value kind {other} in artifact"
        ))),
    }
}

// ============================================================================
// Binary Emitter — pure-Rust inverse of parse_wire_dags
// ============================================================================

/// Emit a slice of `ExecutionDag`s to the wire binary format (v3).
///
/// This is the pure-Rust inverse of [`parse_wire_dags`]: for any set of DAGs,
/// `parse_wire_dags(&emit_wire_dags(dags)?)` reproduces the original DAGs
/// (modulo `HashMap` ordering, which the parser rebuilds).
pub fn emit_wire_dags(dags: &[ExecutionDag]) -> Result<Vec<u8>, CompilerError> {
    let mut w = BinaryWriter::new();

    // Header: version + DAG count
    w.write_u32(WIRE_VERSION);
    w.write_u64(dags.len() as u64);

    for dag in dags {
        emit_single_dag(&mut w, dag)?;
    }

    Ok(w.into_bytes())
}

/// Emit a single `ExecutionDag` to the wire format.
pub fn emit_wire_dag(dag: &ExecutionDag) -> Result<Vec<u8>, CompilerError> {
    emit_wire_dags(std::slice::from_ref(dag))
}

fn emit_single_dag(w: &mut BinaryWriter, dag: &ExecutionDag) -> Result<(), CompilerError> {
    // Module name (empty string for unnamed)
    let module_name = dag.metadata.name.as_deref().unwrap_or("");
    w.write_string(module_name);
    w.write_bool(dag.metadata.is_entry);

    // Parameters
    w.write_u64(dag.metadata.parameters.len() as u64);
    for param in &dag.metadata.parameters {
        w.write_string(&param.name);
        w.write_string(&param.type_name);
    }

    // Nodes
    w.write_u64(dag.nodes.len() as u64);
    for node in &dag.nodes {
        write_node(w, node)?;
    }

    // Edges
    w.write_u64(dag.edges.len() as u64);
    for edge in &dag.edges {
        w.write_u64(edge.from);
        w.write_u64(edge.to);
        w.write_u64(edge.token_id);
        let dep_byte = match edge.dependency_type {
            DependencyType::Data => 0u8,
            DependencyType::Effect => 1u8,
            DependencyType::Control => 2u8,
        };
        w.write_u8(dep_byte);
    }

    // Entry / exit node lists
    w.write_u64(dag.entry_nodes.len() as u64);
    for &id in &dag.entry_nodes {
        w.write_u64(id);
    }
    w.write_u64(dag.exit_nodes.len() as u64);
    for &id in &dag.exit_nodes {
        w.write_u64(id);
    }

    Ok(())
}

fn write_node(w: &mut BinaryWriter, node: &Node) -> Result<(), CompilerError> {
    w.write_u64(node.id);
    let wire_index = node.op_type.to_wire_index().ok_or_else(|| {
        invalid_input_error(format!(
            "Operation {:?} has no wire index and cannot be emitted",
            node.op_type
        ))
    })?;
    w.write_u32(wire_index);

    // Attributes — sort by key for deterministic output
    let mut attrs: Vec<(&String, &Value)> = node.attributes.iter().collect();
    attrs.sort_by_key(|(k, _)| *k);
    w.write_u64(attrs.len() as u64);
    for (key, value) in attrs {
        w.write_string(key);
        write_value(w, value);
    }

    // Input / output tokens
    w.write_u64(node.input_tokens.len() as u64);
    for &tok in &node.input_tokens {
        w.write_u64(tok);
    }
    w.write_u64(node.output_tokens.len() as u64);
    for &tok in &node.output_tokens {
        w.write_u64(tok);
    }

    // Metadata
    w.write_u32(node.metadata.priority);
    match node.metadata.estimated_latency {
        Some(lat) => {
            w.write_bool(true);
            w.write_u64(lat);
        }
        None => {
            w.write_bool(false);
        }
    }

    Ok(())
}

fn write_value(w: &mut BinaryWriter, value: &Value) {
    match value {
        Value::Null => w.write_u8(0),
        Value::Bool(v) => {
            w.write_u8(1);
            w.write_bool(*v);
        }
        Value::Number(Number::Integer(v)) => {
            w.write_u8(2);
            w.write_i64(*v);
        }
        Value::Number(Number::Float(v)) => {
            w.write_u8(3);
            w.write_f64(*v);
        }
        Value::String(v) => {
            w.write_u8(4);
            w.write_string(v);
        }
        Value::Array(values) => {
            w.write_u8(5);
            w.write_u64(values.len() as u64);
            for v in values {
                write_value(w, v);
            }
        }
        Value::Object(map) => {
            w.write_u8(6);
            // Sort keys for deterministic output
            let mut entries: Vec<(&String, &Value)> = map.iter().collect();
            entries.sort_by_key(|(k, _)| *k);
            w.write_u64(entries.len() as u64);
            for (key, val) in entries {
                w.write_string(key);
                write_value(w, val);
            }
        }
        Value::Token(id) => {
            w.write_u8(7);
            w.write_u64(*id);
        }
    }
}

struct BinaryWriter {
    buf: Vec<u8>,
}

impl BinaryWriter {
    fn new() -> Self {
        Self { buf: Vec::new() }
    }

    fn into_bytes(self) -> Vec<u8> {
        self.buf
    }

    fn write_u8(&mut self, v: u8) {
        self.buf.push(v);
    }

    fn write_bool(&mut self, v: bool) {
        self.write_u8(if v { 1 } else { 0 });
    }

    fn write_u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    fn write_u64(&mut self, v: u64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    fn write_i64(&mut self, v: i64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    fn write_f64(&mut self, v: f64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    fn write_string(&mut self, s: &str) {
        self.write_u64(s.len() as u64);
        self.buf.extend_from_slice(s.as_bytes());
    }
}

// ============================================================================
// Binary Reader — parser helpers
// ============================================================================

struct BinaryReader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> BinaryReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    fn read_exact(&mut self, count: usize) -> Result<&'a [u8], CompilerError> {
        if self.pos + count > self.bytes.len() {
            return Err(invalid_input_error(
                "Unexpected end of artifact wire payload".to_string(),
            ));
        }
        let slice = &self.bytes[self.pos..self.pos + count];
        self.pos += count;
        Ok(slice)
    }

    fn read_u8(&mut self) -> Result<u8, CompilerError> {
        Ok(self.read_exact(1)?[0])
    }

    fn read_bool(&mut self) -> Result<bool, CompilerError> {
        Ok(self.read_u8()? != 0)
    }

    fn read_u32(&mut self) -> Result<u32, CompilerError> {
        let mut buf = [0u8; 4];
        buf.copy_from_slice(self.read_exact(4)?);
        Ok(u32::from_le_bytes(buf))
    }

    fn read_u64(&mut self) -> Result<u64, CompilerError> {
        let mut buf = [0u8; 8];
        buf.copy_from_slice(self.read_exact(8)?);
        Ok(u64::from_le_bytes(buf))
    }

    fn read_i64(&mut self) -> Result<i64, CompilerError> {
        let mut buf = [0u8; 8];
        buf.copy_from_slice(self.read_exact(8)?);
        Ok(i64::from_le_bytes(buf))
    }

    fn read_f64(&mut self) -> Result<f64, CompilerError> {
        let mut buf = [0u8; 8];
        buf.copy_from_slice(self.read_exact(8)?);
        Ok(f64::from_le_bytes(buf))
    }

    fn read_string(&mut self) -> Result<String, CompilerError> {
        let len = self.read_u64()? as usize;
        let bytes = self.read_exact(len)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| {
            invalid_input_error("Invalid UTF-8 string in artifact payload".to_string())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_core::types::NodeMetadata;
    use apxm_core::types::execution::{DagMetadata, FlowParameter};

    /// Helper: compare two DAGs field-by-field.
    /// HashMap ordering is non-deterministic so we compare sorted attribute vecs.
    fn assert_dag_eq(a: &ExecutionDag, b: &ExecutionDag) {
        assert_eq!(a.nodes.len(), b.nodes.len(), "node count mismatch");
        for (na, nb) in a.nodes.iter().zip(b.nodes.iter()) {
            assert_eq!(na.id, nb.id, "node id mismatch");
            assert_eq!(
                na.op_type, nb.op_type,
                "op_type mismatch for node {}",
                na.id
            );
            assert_eq!(
                na.input_tokens, nb.input_tokens,
                "input_tokens mismatch for node {}",
                na.id
            );
            assert_eq!(
                na.output_tokens, nb.output_tokens,
                "output_tokens mismatch for node {}",
                na.id
            );
            assert_eq!(
                na.metadata.priority, nb.metadata.priority,
                "priority mismatch for node {}",
                na.id
            );
            assert_eq!(
                na.metadata.estimated_latency, nb.metadata.estimated_latency,
                "latency mismatch for node {}",
                na.id
            );
            // Compare attributes by sorted key
            let mut a_attrs: Vec<_> = na.attributes.iter().collect();
            a_attrs.sort_by_key(|(k, _)| (*k).clone());
            let mut b_attrs: Vec<_> = nb.attributes.iter().collect();
            b_attrs.sort_by_key(|(k, _)| (*k).clone());
            assert_eq!(a_attrs, b_attrs, "attributes mismatch for node {}", na.id);
        }
        assert_eq!(a.edges.len(), b.edges.len(), "edge count mismatch");
        for (ea, eb) in a.edges.iter().zip(b.edges.iter()) {
            assert_eq!(ea.from, eb.from);
            assert_eq!(ea.to, eb.to);
            assert_eq!(ea.token_id, eb.token_id);
            assert_eq!(ea.dependency_type, eb.dependency_type);
        }
        assert_eq!(a.entry_nodes, b.entry_nodes);
        assert_eq!(a.exit_nodes, b.exit_nodes);
        assert_eq!(a.metadata.name, b.metadata.name);
        assert_eq!(a.metadata.is_entry, b.metadata.is_entry);
        assert_eq!(a.metadata.parameters, b.metadata.parameters);
    }

    fn minimal_dag() -> ExecutionDag {
        ExecutionDag {
            nodes: vec![Node {
                id: 1,
                op_type: AISOperationType::InvTool,
                attributes: HashMap::new(),
                input_tokens: vec![],
                output_tokens: vec![100],
                metadata: NodeMetadata {
                    name: None,
                    priority: 0,
                    estimated_latency: None,
                    task_source_id: None,
                },
            }],
            edges: vec![],
            entry_nodes: vec![1],
            exit_nodes: vec![1],
            metadata: DagMetadata {
                name: Some("test".into()),
                is_entry: true,
                parameters: vec![],
            },
        }
    }

    fn complex_dag() -> ExecutionDag {
        let mut attrs = HashMap::new();
        attrs.insert(
            "prompt".to_string(),
            Value::String("hello world".to_string()),
        );
        attrs.insert("temperature".to_string(), Value::Number(Number::Float(0.7)));
        attrs.insert(
            "max_tokens".to_string(),
            Value::Number(Number::Integer(1024)),
        );
        attrs.insert("flag".to_string(), Value::Bool(true));
        attrs.insert("empty".to_string(), Value::Null);
        attrs.insert(
            "tags".to_string(),
            Value::Array(vec![
                Value::String("a".to_string()),
                Value::Number(Number::Integer(42)),
            ]),
        );
        let mut inner_obj = HashMap::new();
        inner_obj.insert("nested_key".to_string(), Value::Bool(false));
        attrs.insert("config".to_string(), Value::Object(inner_obj));
        attrs.insert("tok_ref".to_string(), Value::Token(99));

        ExecutionDag {
            nodes: vec![
                Node {
                    id: 1,
                    op_type: AISOperationType::Ask,
                    attributes: attrs,
                    input_tokens: vec![],
                    output_tokens: vec![10],
                    metadata: NodeMetadata {
                        name: None,
                        priority: 5,
                        estimated_latency: Some(500_000),
                        task_source_id: None,
                    },
                },
                Node {
                    id: 2,
                    op_type: AISOperationType::Merge,
                    attributes: HashMap::new(),
                    input_tokens: vec![10],
                    output_tokens: vec![20],
                    metadata: NodeMetadata {
                        name: None,
                        priority: 0,
                        estimated_latency: None,
                        task_source_id: None,
                    },
                },
                Node {
                    id: 3,
                    op_type: AISOperationType::Print,
                    attributes: HashMap::new(),
                    input_tokens: vec![20],
                    output_tokens: vec![],
                    metadata: NodeMetadata {
                        name: None,
                        priority: 10,
                        estimated_latency: Some(100),
                        task_source_id: None,
                    },
                },
            ],
            edges: vec![
                Edge {
                    from: 1,
                    to: 2,
                    token_id: 10,
                    dependency_type: DependencyType::Data,
                },
                Edge {
                    from: 2,
                    to: 3,
                    token_id: 20,
                    dependency_type: DependencyType::Effect,
                },
            ],
            entry_nodes: vec![1],
            exit_nodes: vec![3],
            metadata: DagMetadata {
                name: Some("MyAgent.main".into()),
                is_entry: true,
                parameters: vec![
                    FlowParameter {
                        name: "topic".into(),
                        type_name: "str".into(),
                    },
                    FlowParameter {
                        name: "depth".into(),
                        type_name: "int".into(),
                    },
                ],
            },
        }
    }

    #[test]
    fn emit_parse_round_trip_minimal() {
        let dag = minimal_dag();
        let bytes = emit_wire_dags(&[dag.clone()]).expect("emit should succeed");
        let parsed = parse_wire_dags(&bytes).expect("parse should succeed");
        assert_eq!(parsed.len(), 1);
        assert_dag_eq(&dag, &parsed[0]);
    }

    #[test]
    fn emit_parse_round_trip_complex() {
        let dag = complex_dag();
        let bytes = emit_wire_dags(&[dag.clone()]).expect("emit should succeed");
        let parsed = parse_wire_dags(&bytes).expect("parse should succeed");
        assert_eq!(parsed.len(), 1);
        assert_dag_eq(&dag, &parsed[0]);
    }

    #[test]
    fn emit_parse_round_trip_multi_dag() {
        let dag1 = minimal_dag();
        let mut dag2 = complex_dag();
        dag2.metadata.name = Some("second_flow".into());
        dag2.metadata.is_entry = false;

        let dags = vec![dag1.clone(), dag2.clone()];
        let bytes = emit_wire_dags(&dags).expect("emit should succeed");
        let parsed = parse_wire_dags(&bytes).expect("parse should succeed");
        assert_eq!(parsed.len(), 2);
        assert_dag_eq(&dag1, &parsed[0]);
        assert_dag_eq(&dag2, &parsed[1]);
    }

    #[test]
    fn emit_parse_round_trip_empty_dags() {
        let bytes = emit_wire_dags(&[]).expect("emit should succeed for empty slice");
        let parsed = parse_wire_dags(&bytes).expect("parse should succeed");
        assert!(parsed.is_empty());
    }

    #[test]
    fn emit_parse_round_trip_unnamed_dag() {
        let mut dag = minimal_dag();
        dag.metadata.name = None;
        let bytes = emit_wire_dags(&[dag.clone()]).expect("emit should succeed");
        let parsed = parse_wire_dags(&bytes).expect("parse should succeed");
        assert_eq!(parsed.len(), 1);
        // Parser converts empty string to None
        assert_dag_eq(&dag, &parsed[0]);
    }

    #[test]
    fn emit_parse_round_trip_all_dependency_types() {
        let dag = ExecutionDag {
            nodes: vec![
                Node {
                    id: 1,
                    op_type: AISOperationType::InvTool,
                    attributes: HashMap::new(),
                    input_tokens: vec![],
                    output_tokens: vec![10, 20, 30],
                    metadata: NodeMetadata::default(),
                },
                Node {
                    id: 2,
                    op_type: AISOperationType::Ask,
                    attributes: HashMap::new(),
                    input_tokens: vec![10, 20, 30],
                    output_tokens: vec![],
                    metadata: NodeMetadata::default(),
                },
            ],
            edges: vec![
                Edge {
                    from: 1,
                    to: 2,
                    token_id: 10,
                    dependency_type: DependencyType::Data,
                },
                Edge {
                    from: 1,
                    to: 2,
                    token_id: 20,
                    dependency_type: DependencyType::Effect,
                },
                Edge {
                    from: 1,
                    to: 2,
                    token_id: 30,
                    dependency_type: DependencyType::Control,
                },
            ],
            entry_nodes: vec![1],
            exit_nodes: vec![2],
            metadata: DagMetadata::default(),
        };
        let bytes = emit_wire_dags(&[dag.clone()]).expect("emit should succeed");
        let parsed = parse_wire_dags(&bytes).expect("parse should succeed");
        assert_eq!(parsed.len(), 1);
        assert_dag_eq(&dag, &parsed[0]);
    }

    #[test]
    fn emit_parse_round_trip_all_wire_indexed_ops() {
        for (i, &(_, op)) in AISOperationType::wire_indexed_operations()
            .iter()
            .enumerate()
        {
            let dag = ExecutionDag {
                nodes: vec![Node {
                    id: (i + 1) as u64,
                    op_type: op,
                    attributes: HashMap::new(),
                    input_tokens: vec![],
                    output_tokens: vec![],
                    metadata: NodeMetadata::default(),
                }],
                edges: vec![],
                entry_nodes: vec![(i + 1) as u64],
                exit_nodes: vec![(i + 1) as u64],
                metadata: DagMetadata::default(),
            };
            let bytes = emit_wire_dags(&[dag.clone()])
                .unwrap_or_else(|e| panic!("emit failed for {:?}: {e}", op));
            let parsed = parse_wire_dags(&bytes)
                .unwrap_or_else(|e| panic!("parse failed for {:?}: {e}", op));
            assert_eq!(parsed.len(), 1);
            assert_eq!(
                parsed[0].nodes[0].op_type, op,
                "op_type mismatch for {:?}",
                op
            );
        }
    }

    #[test]
    fn emit_single_dag_convenience() {
        let dag = minimal_dag();
        let bytes = emit_wire_dag(&dag).expect("emit_wire_dag should succeed");
        let parsed = parse_wire_dag(&bytes).expect("parse_wire_dag should succeed");
        assert_dag_eq(&dag, &parsed);
    }

    #[test]
    fn emit_deterministic_bytes() {
        // Emit the same DAG twice — the bytes must be identical
        let dag = complex_dag();
        let bytes1 = emit_wire_dags(&[dag.clone()]).expect("first emit");
        let bytes2 = emit_wire_dags(&[dag]).expect("second emit");
        assert_eq!(bytes1, bytes2, "emit must be deterministic");
    }
}
