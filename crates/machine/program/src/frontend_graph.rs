//! `apxm.frontend-graph.v1` — closed consumer types and verification.
//!
//! The FrontendGraph is the language-neutral typed source graph recorded
//! equivalently by the Python and TypeScript source-first frontends. It records
//! typed declarations and bindings, typed values, blocks, lexical
//! regions, typed data edges, discriminated typed source intents (call and
//! control), explicit context flow, static Hook bindings, exact requirements, and
//! a source map. It carries typed source intent only: it never contains an
//! AIR/AIS operation string, an `ais.*` kind, an untyped operand bag, runtime
//! placement, or credentials. Rust alone selects AIS operations from these
//! intents. `deny_unknown_fields` rejects any out-of-shape field at decode.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::diagnostic::{Diagnostic, DiagnosticCode, Verdict, schema_violation};
use crate::grammar::{is_digest, is_identifier};
use crate::source_map::{SourceLanguage, SourceMap};

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

/// The closed set of typed declaration roles. Friendly names
/// (Context/Model/Tool/Capability/Event) map to these bound nodes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeclKind {
    Context,
    ModelBinding,
    ToolBinding,
    ToolHandler,
    CapabilityBinding,
    CapabilityHandler,
    EventType,
}

/// The closed set of typed parameter roles.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParameterRole {
    AgentFacade,
    Input,
    Context,
    Ordinary,
}

/// The closed set of typed value origins. Every value has exactly one origin.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValueOrigin {
    Parameter,
    CallResult,
    BlockArgument,
    ContextValue,
    Literal,
    ResumeInput,
}

/// The closed set of lexical region roles. This is source structure, not an AIS
/// op kind; Rust maps it to structural AIS.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegionRole {
    FunctionBody,
    ConditionalArm,
    LoopBody,
    TaskScope,
    TaskChild,
    TryBody,
    CatchBody,
}

/// The closed set of discriminated typed effect intents. Rust selects exactly one
/// of the five AIS semantic operations from each.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntentKind {
    ModelInvocation,
    ToolInvocation,
    CapabilityInvocation,
    AgentCreation,
    AgentInvocation,
    EventWait,
}

/// The closed receiver-kind set for an agent invocation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReceiverKind {
    ProgramRef,
    ProgramInstanceRef,
}

/// The closed set of discriminated language-neutral structural intents. Rust
/// constructs the closed structural AIS family from these; source never names
/// `ais.loop` or any `ais.*` kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlKind {
    Conditional,
    Switch,
    Loop,
    TaskGroup,
    TryCatch,
    Throw,
    Yield,
    Return,
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

/// One typed source declaration or binding.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Declaration {
    pub decl_id: String,
    pub decl_kind: DeclKind,
    pub input_type_ref: String,
    pub output_type_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handler_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_default_present: Option<bool>,
}

/// One typed function parameter.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Parameter {
    pub value_id: String,
    pub type_ref: String,
    pub role: ParameterRole,
}

/// A typed entrypoint or helper function with a body region.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FunctionDef {
    pub function_id: String,
    pub parameters: Vec<Parameter>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_type_ref: Option<String>,
    pub body_region_id: String,
    pub is_entrypoint: bool,
}

/// One typed value with exactly one typed origin.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Value {
    pub value_id: String,
    pub type_ref: String,
    pub origin: ValueOrigin,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_id: Option<String>,
}

/// One basic block with typed block arguments, contained in a region.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Block {
    pub block_id: String,
    pub region_id: String,
    pub block_arguments: Vec<String>,
    pub execution_order: u32,
}

/// One lexical region. `region_role` is source structure, not an AIS op kind.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Region {
    pub region_id: String,
    pub region_role: RegionRole,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_region_id: Option<String>,
    pub execution_order: u32,
}

/// A typed def-use edge from a producing value to a consuming operand slot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DataEdge {
    pub from_value: String,
    pub to_consumer: String,
    pub consumer_slot: String,
}

/// One discriminated typed effect intent. `intent_kind` names the source role;
/// Rust maps it to an AIR/AIS operation. There is no AIR/AIS string here.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CallIntent {
    pub node_id: String,
    pub intent_kind: IntentKind,
    pub parent_region_id: String,
    pub execution_order: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binding_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receiver_kind: Option<ReceiverKind>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub operand_values: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_value: Option<String>,
}

/// One discriminated language-neutral structural intent. Rust constructs
/// structural AIS from these.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlIntent {
    pub node_id: String,
    pub control_kind: ControlKind,
    pub parent_region_id: String,
    pub execution_order: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub body_region_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub operand_values: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_value: Option<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_schema_present: Option<bool>,
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
    pub declarations: Vec<Declaration>,
    pub functions: Vec<FunctionDef>,
    pub values: Vec<Value>,
    pub blocks: Vec<Block>,
    pub regions: Vec<Region>,
    pub data_edges: Vec<DataEdge>,
    pub call_intents: Vec<CallIntent>,
    pub control_intents: Vec<ControlIntent>,
    pub context_flow: Vec<ContextEdge>,
    pub hook_bindings: Vec<HookBinding>,
    pub capability_requirements: Vec<CapabilityRequirement>,
    pub model_requirements: Vec<ModelRequirement>,
    pub source_map: SourceMap,
}

impl FrontendGraph {
    /// Verify a decoded FrontendGraph, producing deterministic closed
    /// diagnostics. The kind, role, origin, scope, phase, and return-mode
    /// closures are guaranteed by decode; this adds grammar, digest, unique-id,
    /// single-origin, and containment checks over the typed region/block/intent
    /// structure.
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

        // Intent node ids (call and control) share one namespace and must be
        // unique contract identifiers.
        let mut seen_nodes: HashSet<&str> = HashSet::new();
        for intent in &self.call_intents {
            check_identifier(&mut verdict, &intent.node_id, "call intent node_id");
            if !seen_nodes.insert(intent.node_id.as_str()) {
                verdict.push(Diagnostic::new(
                    DiagnosticCode::DuplicateNodeId,
                    intent.node_id.clone(),
                    "intent node_id is not unique",
                ));
            }
        }
        for intent in &self.control_intents {
            check_identifier(&mut verdict, &intent.node_id, "control intent node_id");
            if !seen_nodes.insert(intent.node_id.as_str()) {
                verdict.push(Diagnostic::new(
                    DiagnosticCode::DuplicateNodeId,
                    intent.node_id.clone(),
                    "intent node_id is not unique",
                ));
            }
        }

        // Region ids form the containment tree and must be unique identifiers.
        let mut seen_regions: HashSet<&str> = HashSet::new();
        for region in &self.regions {
            check_identifier(&mut verdict, &region.region_id, "region region_id");
            if !seen_regions.insert(region.region_id.as_str()) {
                verdict.push(Diagnostic::new(
                    DiagnosticCode::DuplicateRegionId,
                    region.region_id.clone(),
                    "region region_id is not unique",
                ));
            }
        }

        // Every value carries one typed origin and a unique identifier.
        let mut seen_values: HashSet<&str> = HashSet::new();
        for value in &self.values {
            check_identifier(&mut verdict, &value.value_id, "value value_id");
            check_identifier(&mut verdict, &value.type_ref, "value type_ref");
            if !seen_values.insert(value.value_id.as_str()) {
                verdict.push(Diagnostic::new(
                    DiagnosticCode::SchemaViolation,
                    value.value_id.clone(),
                    "value value_id is not unique",
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
        .regions
        .iter()
        .map(|region| region.region_id.as_str())
        .collect();
    let mut positions: HashSet<(Option<&str>, u32)> = HashSet::new();

    for region in &graph.regions {
        if let Some(parent) = region.parent_region_id.as_deref()
            && !regions.contains(parent)
        {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                region.region_id.clone(),
                "region parent_region_id does not reference a declared region",
            ));
        }
        if !positions.insert((region.parent_region_id.as_deref(), region.execution_order)) {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                region.region_id.clone(),
                "sibling regions have duplicate execution_order",
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
                    "region containment contains a cycle",
                ));
                break;
            }
            cursor = graph
                .regions
                .iter()
                .find(|candidate| candidate.region_id == parent)
                .and_then(|candidate| candidate.parent_region_id.as_deref());
        }
    }

    // Each intent's parent_region_id references a declared region, and no two
    // siblings (region/block or intent) collide on execution_order in a parent.
    for intent in &graph.call_intents {
        if !regions.contains(intent.parent_region_id.as_str()) {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                intent.node_id.clone(),
                "call intent parent_region_id does not reference a declared region",
            ));
        }
        if !positions.insert((
            Some(intent.parent_region_id.as_str()),
            intent.execution_order,
        )) {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                intent.node_id.clone(),
                "intent siblings have duplicate execution_order",
            ));
        }
    }
    for intent in &graph.control_intents {
        if !regions.contains(intent.parent_region_id.as_str()) {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                intent.node_id.clone(),
                "control intent parent_region_id does not reference a declared region",
            ));
        }
        if !positions.insert((
            Some(intent.parent_region_id.as_str()),
            intent.execution_order,
        )) {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                intent.node_id.clone(),
                "intent siblings have duplicate execution_order",
            ));
        }
    }

    // Every block is contained in a declared region.
    for block in &graph.blocks {
        check_identifier(verdict, &block.block_id, "block block_id");
        if !regions.contains(block.region_id.as_str()) {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                block.block_id.clone(),
                "block region_id does not reference a declared region",
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
