//! `op-spec.v1` catalog generation for the five public semantic operations.

use super::definitions::{AIS_OPERATIONS, SemanticOpKind, WIRE_INDEXED_OPERATIONS};
use serde::Serialize;

pub const OP_SPEC_SCHEMA_VERSION: &str = "apxm.op-spec.v1";
pub const OP_SPEC_VECTORS_SCHEMA_VERSION: &str = "apxm.op-spec-vectors.v1";
pub const OP_SPEC_CATALOG_FILE: &str = "op-spec.v1.json";
pub const OP_SPEC_VECTORS_FILE: &str = "op-spec.vectors.v1.json";

fn wire_index_for(op: SemanticOpKind) -> Option<u32> {
    WIRE_INDEXED_OPERATIONS
        .iter()
        .find(|(_, candidate)| *candidate == op)
        .map(|(index, _)| *index)
}

#[derive(Serialize)]
struct OpSpecField {
    name: &'static str,
    required: bool,
    description: &'static str,
    ref_type: Option<&'static str>,
}

#[derive(Serialize)]
struct OpSpecEntry {
    op: SemanticOpKind,
    rust_variant: &'static str,
    name: &'static str,
    category: super::category::OperationCategory,
    description: &'static str,
    long_description: &'static str,
    latency: super::definitions::OperationLatency,
    wire_index: Option<u32>,
    is_pseudo_op: bool,
    min_inputs: u32,
    produces_output: bool,
    needs_submission: bool,
    fields: Vec<OpSpecField>,
    example_json: Option<&'static str>,
}

#[derive(Serialize)]
struct OpSpecCatalog {
    schema_version: &'static str,
    generated_from: &'static str,
    total_operations: usize,
    pseudo_op_count: usize,
    public_semantic_operation_count: usize,
    operations: Vec<OpSpecEntry>,
}

fn rust_variant_name(op: SemanticOpKind) -> &'static str {
    match op {
        SemanticOpKind::ModelCall => "ModelCall",
        SemanticOpKind::CapabilityInvoke => "CapabilityInvoke",
        SemanticOpKind::ProgramNew => "ProgramNew",
        SemanticOpKind::ProgramInvoke => "ProgramInvoke",
        SemanticOpKind::AwaitEvent => "AwaitEvent",
    }
}

fn build_entries() -> Vec<OpSpecEntry> {
    AIS_OPERATIONS
        .iter()
        .map(|spec| OpSpecEntry {
            op: spec.op_type,
            rust_variant: rust_variant_name(spec.op_type),
            name: spec.name,
            category: spec.category,
            description: spec.description,
            long_description: spec.long_description,
            latency: spec.latency,
            wire_index: wire_index_for(spec.op_type),
            is_pseudo_op: false,
            min_inputs: spec.min_inputs,
            produces_output: spec.produces_output,
            needs_submission: spec.needs_submission,
            fields: spec
                .fields
                .iter()
                .map(|field| OpSpecField {
                    name: field.name,
                    required: field.required,
                    description: field.description,
                    ref_type: field.ref_type.map(|r| r.label()),
                })
                .collect(),
            example_json: spec.example_json,
        })
        .collect()
}

pub fn generate_op_spec_catalog() -> String {
    let entries = build_entries();
    let catalog = OpSpecCatalog {
        schema_version: OP_SPEC_SCHEMA_VERSION,
        generated_from: "crates/machine/ais/src/operations/definitions.rs",
        total_operations: entries.len(),
        pseudo_op_count: 0,
        public_semantic_operation_count: entries.len(),
        operations: entries,
    };
    let mut json = serde_json::to_string_pretty(&catalog).expect("serialize op-spec catalog");
    json.push('\n');
    json
}

#[derive(Serialize)]
struct VectorFixture {
    op: SemanticOpKind,
    example_json: &'static str,
    bytes: usize,
}

#[derive(Serialize)]
struct OpSpecVectors {
    schema_version: &'static str,
    generated_from: &'static str,
    fixtures: Vec<VectorFixture>,
}

pub fn generate_op_spec_vectors() -> String {
    let fixtures: Vec<VectorFixture> = AIS_OPERATIONS
        .iter()
        .filter_map(|spec| {
            spec.example_json.map(|example| VectorFixture {
                op: spec.op_type,
                example_json: example,
                bytes: example.len(),
            })
        })
        .collect();
    let vectors = OpSpecVectors {
        schema_version: OP_SPEC_VECTORS_SCHEMA_VERSION,
        generated_from: "crates/machine/ais/src/operations/definitions.rs",
        fixtures,
    };
    let mut json = serde_json::to_string_pretty(&vectors).expect("serialize op-spec vectors");
    json.push('\n');
    json
}

pub fn render_op_spec_files() -> [(&'static str, String); 2] {
    [
        (OP_SPEC_CATALOG_FILE, generate_op_spec_catalog()),
        (OP_SPEC_VECTORS_FILE, generate_op_spec_vectors()),
    ]
}

#[cfg(test)]
mod drift_gate {
    use super::*;
    use std::path::PathBuf;

    fn generated_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("generated")
    }

    #[test]
    fn op_spec_catalog_matches_definitions() {
        let path = generated_dir().join(OP_SPEC_CATALOG_FILE);
        let committed = std::fs::read_to_string(&path).unwrap_or_else(|e| {
            panic!(
                "missing committed op-spec catalog at {} ({e}); run `apxm codegen op-spec`",
                path.display()
            )
        });
        let fresh = generate_op_spec_catalog();
        assert_eq!(
            committed, fresh,
            "op-spec.v1.json is stale relative to AIS_OPERATIONS in definitions.rs"
        );
    }

    #[test]
    fn op_spec_vectors_match_definitions() {
        let path = generated_dir().join(OP_SPEC_VECTORS_FILE);
        let committed = std::fs::read_to_string(&path).unwrap_or_else(|e| {
            panic!(
                "missing committed op-spec vectors at {} ({e}); run `apxm codegen op-spec`",
                path.display()
            )
        });
        let fresh = generate_op_spec_vectors();
        assert_eq!(
            committed, fresh,
            "op-spec.vectors.v1.json is stale relative to AIS_OPERATIONS in definitions.rs"
        );
    }

    #[test]
    fn catalogue_reports_exactly_five_public_semantic_operations() {
        let entries = build_entries();
        assert_eq!(entries.len(), 5);
        assert!(entries.iter().all(|entry| !entry.is_pseudo_op));
        let wires: Vec<_> = entries
            .iter()
            .map(|entry| entry.op.wire())
            .collect();
        assert_eq!(
            wires,
            vec![
                "model.call",
                "capability.invoke",
                "program.new",
                "program.invoke",
                "await.event"
            ]
        );
    }
}
