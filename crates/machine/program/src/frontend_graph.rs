//! `apxm.frontend-graph.v1` — closed consumer types and verification.
//!
//! The FrontendGraph is the language-neutral graph recorded equivalently by the
//! Python and TypeScript authoring frontends. It records program definitions,
//! imported program references, the five semantic operations, structural
//! control flow, context flow, static Hook bindings, and a source map. It never
//! records raw AIR text, runtime placement, or credentials: `deny_unknown_fields`
//! rejects any such field at decode.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use serde_json::Map;

use crate::air::{SemanticOpKind, StructuralOpKind};
use crate::diagnostic::{Diagnostic, DiagnosticCode, Verdict, schema_violation};
use crate::grammar::{is_digest, is_identifier};
use crate::source_map::{RegionAnnotationKind, SourceLanguage, SourceMap};

/// The single accepted `schema_version` for a FrontendGraph.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FrontendGraphVersion {
    #[serde(rename = "apxm.frontend-graph.v1")]
    V1,
}

/// The closed Hook scope set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookScope {
    Agent,
    Loop,
    Node,
    Model,
    Capability,
}

/// The closed Hook phase set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookPhase {
    Before,
    After,
}

/// The closed Hook return-mode set. A Hook either observes or returns a
/// statically declared replacement result; implicit context mutation is not a
/// return mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookReturnMode {
    Observe,
    ReplaceResult,
}

/// A program authored in this graph.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgramDefinition {
    pub program_id: String,
    pub entrypoint: String,
    pub input_type_ref: String,
    pub output_type_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_type_ref: Option<String>,
    pub has_default_context: bool,
}

/// A digest-pinned reference to an externally built program.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportedProgramRef {
    pub program_ref: String,
    pub artifact_digest: String,
    pub entrypoint: String,
    pub target_agent_identity_requirement: String,
}

/// One recorded semantic operation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SemanticOperation {
    pub node_id: String,
    pub op: SemanticOpKind,
    pub parent_region_id: String,
    pub execution_order: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operands: Option<Map<String, serde_json::Value>>,
}

/// One structural control-flow region.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StructuralRegion {
    pub region_id: String,
    pub kind: StructuralOpKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_region_id: Option<String>,
    pub execution_order: u32,
}

/// One explicit context-flow edge.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextEdge {
    pub from_node: String,
    pub to_node: String,
    pub context_type_ref: String,
}

/// One statically compiled Hook binding.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HookBinding {
    pub hook_id: String,
    pub scope: HookScope,
    pub phase: HookPhase,
    pub target_selector: String,
    pub declaration_order: u32,
    pub handler_ref: String,
    pub handler_digest: String,
    pub input_type_ref: String,
    pub output_type_ref: String,
    pub return_mode: HookReturnMode,
}

/// A declared Capability requirement.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityRequirement {
    pub capability_ref: String,
}

/// A declared model requirement referencing exactly one target.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelRequirement {
    pub model_target_ref: String,
}

/// A decoded FrontendGraph.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrontendGraph {
    pub schema_version: FrontendGraphVersion,
    pub source_language: SourceLanguage,
    pub program_definitions: Vec<ProgramDefinition>,
    pub imported_program_refs: Vec<ImportedProgramRef>,
    pub semantic_operations: Vec<SemanticOperation>,
    pub structural_regions: Vec<StructuralRegion>,
    pub context_flow: Vec<ContextEdge>,
    pub hook_bindings: Vec<HookBinding>,
    pub capability_requirements: Vec<CapabilityRequirement>,
    pub model_requirements: Vec<ModelRequirement>,
    pub source_map: SourceMap,
}

impl FrontendGraph {
    /// Verify a decoded FrontendGraph, producing deterministic closed
    /// diagnostics. The op, kind, scope, phase, and return-mode closures are
    /// guaranteed by decode; this adds grammar, digest, and unique-id checks.
    #[must_use]
    pub fn verify(&self) -> Verdict {
        let mut verdict = Verdict::accepted();

        if self.program_definitions.is_empty() {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                "program_definitions",
                "a FrontendGraph declares at least one program",
            ));
        }

        let mut seen_nodes: HashSet<&str> = HashSet::new();
        for op in &self.semantic_operations {
            check_identifier(&mut verdict, &op.node_id, "semantic operation node_id");
            if !seen_nodes.insert(op.node_id.as_str()) {
                verdict.push(Diagnostic::new(
                    DiagnosticCode::DuplicateNodeId,
                    op.node_id.clone(),
                    "semantic operation node_id is not unique",
                ));
            }
        }

        let mut seen_regions: HashSet<&str> = HashSet::new();
        for region in &self.structural_regions {
            check_identifier(
                &mut verdict,
                &region.region_id,
                "structural region region_id",
            );
            if !seen_regions.insert(region.region_id.as_str()) {
                verdict.push(Diagnostic::new(
                    DiagnosticCode::DuplicateRegionId,
                    region.region_id.clone(),
                    "structural region region_id is not unique",
                ));
            }
        }

        collect_containment_diagnostics(&mut verdict, self);
        for import in &self.imported_program_refs {
            check_identifier(&mut verdict, &import.program_ref, "imported program_ref");
            check_digest(&mut verdict, &import.artifact_digest, &import.program_ref);
        }

        for hook in &self.hook_bindings {
            check_identifier(&mut verdict, &hook.hook_id, "hook_id");
            check_digest(&mut verdict, &hook.handler_digest, &hook.hook_id);
        }

        self.source_map.collect(&mut verdict);
        verdict.finish()
    }
}

fn collect_containment_diagnostics(verdict: &mut Verdict, graph: &FrontendGraph) {
    let regions: HashSet<&str> = graph
        .structural_regions
        .iter()
        .map(|region| region.region_id.as_str())
        .collect();
    let mut positions: HashSet<(Option<&str>, u32)> = HashSet::new();

    for region in &graph.structural_regions {
        if let Some(parent) = region.parent_region_id.as_deref()
            && !regions.contains(parent)
        {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                region.region_id.clone(),
                "structural parent_region_id does not reference a declared region",
            ));
        }
        if !positions.insert((region.parent_region_id.as_deref(), region.execution_order)) {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                region.region_id.clone(),
                "structural siblings have duplicate execution_order",
            ));
        }

        let mut cursor = region.parent_region_id.as_deref();
        let mut ancestors = HashSet::new();
        ancestors.insert(region.region_id.as_str());
        while let Some(parent) = cursor {
            if !ancestors.insert(parent) {
                verdict.push(Diagnostic::new(
                    DiagnosticCode::SchemaViolation,
                    region.region_id.clone(),
                    "structural containment contains a cycle",
                ));
                break;
            }
            cursor = graph
                .structural_regions
                .iter()
                .find(|candidate| candidate.region_id == parent)
                .and_then(|candidate| candidate.parent_region_id.as_deref());
        }
    }

    for operation in &graph.semantic_operations {
        if !regions.contains(operation.parent_region_id.as_str()) {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                operation.node_id.clone(),
                "semantic parent_region_id does not reference a declared region",
            ));
        }
        if !positions.insert((
            Some(operation.parent_region_id.as_str()),
            operation.execution_order,
        )) {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                operation.node_id.clone(),
                "structural and semantic siblings have duplicate execution_order",
            ));
        }
    }

    let loop_ids: HashSet<&str> = graph
        .structural_regions
        .iter()
        .filter(|region| region.kind == StructuralOpKind::Loop)
        .map(|region| region.region_id.as_str())
        .collect();
    let mut annotated_loop_ids = HashSet::new();
    for annotation in &graph.source_map.region_annotations {
        if annotation.annotation != RegionAnnotationKind::StructuralLoop {
            continue;
        }
        if !loop_ids.contains(annotation.region_id.as_str())
            || !annotated_loop_ids.insert(annotation.region_id.as_str())
        {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                annotation.region_id.clone(),
                "structural_loop annotation must identify exactly one declared loop",
            ));
        }
    }
    for loop_id in loop_ids {
        if !annotated_loop_ids.contains(loop_id) {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                loop_id,
                "structural loop is missing its generic source-map identity",
            ));
        }
    }
}

fn check_identifier(verdict: &mut Verdict, value: &str, what: &str) {
    if !is_identifier(value) {
        verdict.push(Diagnostic::new(
            DiagnosticCode::InvalidIdentifier,
            value.to_string(),
            format!("{what} is not a contract identifier"),
        ));
    }
}

fn check_digest(verdict: &mut Verdict, value: &str, location: &str) {
    if !is_digest(value) {
        verdict.push(Diagnostic::new(
            DiagnosticCode::InvalidDigest,
            location.to_string(),
            "digest is not a lowercase sha256 value",
        ));
    }
}

/// Verify a FrontendGraph presented as JSON, failing closed on decode errors.
#[must_use]
pub fn verify_frontend_graph_json(value: &serde_json::Value) -> Verdict {
    match serde_json::from_value::<FrontendGraph>(value.clone()) {
        Ok(graph) => graph.verify(),
        Err(error) => schema_violation("frontend_graph", &error),
    }
}
