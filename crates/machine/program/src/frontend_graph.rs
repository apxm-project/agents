//! `apxm.frontend-graph` — closed consumer types and verification.
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

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

const MAX_SAFE_PREDICATE_INTEGER: u64 = 9_007_199_254_740_991;

pub use apxm_ais::permissions::PermissionDecision;

use crate::diagnostic::{Diagnostic, DiagnosticCode, Verdict, schema_violation};
use crate::grammar::{is_digest, is_identifier};
use crate::source_map::{SourceLanguage, SourceMap};

/// The single accepted `schema_version` for a FrontendGraph.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FrontendGraphVersion {
    #[serde(rename = "apxm.frontend-graph")]
    V2,
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

/// The closed Hook return-mode set.
///
/// This is derived from the captured Hook body, not asserted by the author: a
/// body that assigns Context is [`HookReturnMode::ReplaceResult`], a body that
/// mutates nothing is [`HookReturnMode::Observe`]. Execution enforces it — an
/// observing Hook whose handler returns a replacement is refused.
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
    CapabilityBinding,
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
    /// The captured body of one static Hook. A Hook is executable Agent Program
    /// structure, so its body is ordinary captured source held in its own
    /// region rather than an opaque handler the artifact never describes.
    HookBody,
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

/// The closed comparison family supported by authored structural control.
/// Frontends lower source predicates into this identity-neutral form instead of
/// preserving language AST or asking the runtime to interpret source code.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PredicateComparator {
    Truthy,
    Equals,
    NotEquals,
}

/// A closed typed scalar literal used by structural predicate comparison.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "scalar_type", content = "value", rename_all = "snake_case")]
pub enum PredicateLiteral {
    Boolean(bool),
    String(String),
    Integer(i64),
    Null,
}

/// A language-neutral typed predicate over one produced SSA value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlPredicate {
    pub root_value_id: String,
    pub property_path: Vec<String>,
    pub comparator: PredicateComparator,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub literal: Option<PredicateLiteral>,
}

/// The source declaration's role, not a separate execution engine or authority.
/// Unclassified low-level programs omit this projection; a model call alone
/// does not turn a Workflow into an Agent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProgramAuthoring {
    Workflow {},
    Agent { primary_model_ref: String },
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
    pub input_contract: Option<EntrypointInputContract>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<crate::input_schema::EntrypointInputSchema>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authoring: Option<ProgramAuthoring>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_type_ref: Option<String>,
    pub has_default_context: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_context: Option<ValueExpression>,
}

/// A compiler-owned proof that the typed entrypoint input admits `{}`.
/// Consumers requiring unattended empty JSON input must match this value;
/// absence remains ineligible for that path.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntrypointInputContract {
    AcceptsEmptyObject,
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
    /// Closed, lossless representation of an authored pure value expression.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expression: Option<ValueExpression>,
}

/// One field in an authored object expression.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValueField {
    pub name: String,
    pub value: ValueExpression,
}

/// Closed pure expression grammar used to assemble effect operands and Context.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ValueExpression {
    Ssa {
        value_id: String,
    },
    Context {
        property_path: Vec<String>,
    },
    Projection {
        root: Box<ValueExpression>,
        property_path: Vec<String>,
    },
    Object {
        fields: Vec<ValueField>,
    },
    Array {
        items: Vec<ValueExpression>,
    },
    String {
        value: String,
    },
    Integer {
        value: i64,
    },
    Boolean {
        value: bool,
    },
    Null,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub predicate: Option<ControlPredicate>,
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
    pub value_id: String,
}

/// One statically compiled Hook binding.
///
/// `body_region_id` names the region holding the Hook's captured body. A Hook's
/// `CountTokens` or `Compactor` call is an ordinary `capability.invoke` or
/// `model.call` inside that region, so context assembly, token budgets, and
/// compaction stay measurable workflow structure. Nothing here is a digest
/// reference the runtime resolves out of band.
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
    pub body_region_id: String,
    /// The typed Context value the captured body assigns, carried exactly when
    /// `return_mode` is [`HookReturnMode::ReplaceResult`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assigned_context_value_id: Option<String>,
}

/// One declared Capability requirement.
///
/// A graph carries one record per authored declaration, not one per
/// `capability_ref`: the same capability may be declared both as a Tool and as
/// a plain Capability, and both declarations survive.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityRequirement {
    pub capability_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_schema_present: Option<bool>,
    /// The permission decision the author requested for this capability.
    ///
    /// It is the program's *request*, recorded and digest-bound as authored,
    /// and it confers no authority: the resolution layer stack may only narrow
    /// it, and no layer may widen it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_permission: Option<PermissionDecision>,
}

/// A declared model requirement referencing exactly one target.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelRequirement {
    pub model_target_ref: String,
}

/// The ceiling one authored inline instruction body may reach.
///
/// It is the ceiling the skill-reading capability already enforces on a body it
/// loads: an inline skill is the same trusted context landing in the same
/// model window, so admitting a larger one here would only move the failure
/// from authoring to execution.
pub const MAX_INSTRUCTION_BYTES: usize = 128 * 1024;

/// Where an authored skill's instructions live.
///
/// The two branches reach the artifact digest by different routes, which is why
/// they are discriminated rather than merged. An [`SkillInstructionSource::Entry`]
/// names a package file hashed into the carrying package's integrity chain; an
/// [`SkillInstructionSource::Inline`] carries the text, which is already inside
/// the source bundle. Neither restates the other's anchor.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SkillInstructionSource {
    Entry { path: String },
    Inline { text: String },
}

/// One authored Agent Skill declaration.
///
/// This is the program's statement, not the resolved `apxm.package-local-skill`
/// document: a packager computes digests and byte counts, and a graph records
/// what the author wrote. The authority to load it is an ordinary
/// [`CapabilityRequirement`] on the skill-reading capability, so it appears
/// there rather than here.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillRequirement {
    pub skill_id: String,
    pub instruction_source: SkillInstructionSource,
}

impl SkillRequirement {
    /// The one package path the folder contract recognizes for this skill's
    /// instruction file. Both the authoring frontends and this verifier derive
    /// the path from the id rather than accepting an independent second
    /// spelling of where the instructions live.
    #[must_use]
    pub fn entry_path_for(skill_id: &str) -> String {
        format!("skills/{skill_id}/SKILL.md")
    }
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
    pub skill_requirements: Vec<SkillRequirement>,
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

        // The decision itself is closed, so decode already rejected anything
        // outside the vocabulary. What decode cannot reject is a decision that
        // structurally carries a reason and says nothing, which would put an
        // empty explanation into the digest-bound source bundle.
        for requirement in &self.capability_requirements {
            if let Some(permission) = &requirement.requested_permission
                && permission
                    .reason()
                    .is_some_and(|reason| reason.trim().is_empty())
            {
                verdict.push(Diagnostic::new(
                    DiagnosticCode::SchemaViolation,
                    requirement.capability_ref.clone(),
                    "capability requirement requested_permission carries an empty reason",
                ));
            }
        }

        collect_skill_diagnostics(&mut verdict, self);
        collect_containment_diagnostics(&mut verdict, self);
        collect_typed_link_diagnostics(&mut verdict, self);

        for import in &self.imported_program_refs {
            check_identifier(&mut verdict, &import.program_ref, "imported program_ref");
            check_digest(&mut verdict, &import.artifact_digest, &import.program_ref);
        }

        collect_hook_diagnostics(&mut verdict, self);

        self.source_map.collect(&mut verdict);
        verdict.finish()
    }
}

/// Verify the typed cross references that turn the JSON DTO into a closed
/// FrontendGraph rather than a collection of independently well-formed lists.
/// Every value, declaration, region, function, data edge, and source intent is
/// resolved before Rust selects an AIS operation.
fn collect_typed_link_diagnostics(verdict: &mut Verdict, graph: &FrontendGraph) {
    let declarations: HashMap<&str, &Declaration> = graph
        .declarations
        .iter()
        .map(|declaration| (declaration.decl_id.as_str(), declaration))
        .collect();
    let values: HashMap<&str, &Value> = graph
        .values
        .iter()
        .map(|value| (value.value_id.as_str(), value))
        .collect();
    let functions: HashMap<&str, &FunctionDef> = graph
        .functions
        .iter()
        .map(|function| (function.function_id.as_str(), function))
        .collect();
    let blocks: HashMap<&str, &Block> = graph
        .blocks
        .iter()
        .map(|block| (block.block_id.as_str(), block))
        .collect();
    let regions: HashSet<&str> = graph
        .regions
        .iter()
        .map(|region| region.region_id.as_str())
        .collect();
    let region_definitions: HashMap<&str, &Region> = graph
        .regions
        .iter()
        .map(|region| (region.region_id.as_str(), region))
        .collect();
    let call_intents: HashMap<&str, &CallIntent> = graph
        .call_intents
        .iter()
        .map(|intent| (intent.node_id.as_str(), intent))
        .collect();
    let control_intents: HashMap<&str, &ControlIntent> = graph
        .control_intents
        .iter()
        .map(|intent| (intent.node_id.as_str(), intent))
        .collect();

    check_unique_ids(
        verdict,
        graph.declarations.iter().map(|item| item.decl_id.as_str()),
        "declaration",
    );
    check_unique_ids(
        verdict,
        graph.functions.iter().map(|item| item.function_id.as_str()),
        "function",
    );
    check_unique_ids(
        verdict,
        graph.blocks.iter().map(|item| item.block_id.as_str()),
        "block",
    );
    check_unique_ids(
        verdict,
        graph
            .program_definitions
            .iter()
            .map(|item| item.program_id.as_str()),
        "program definition",
    );
    check_unique_ids(
        verdict,
        graph
            .imported_program_refs
            .iter()
            .map(|item| item.program_ref.as_str()),
        "imported program reference",
    );

    let mut entrypoints = 0usize;
    for function in &graph.functions {
        check_identifier(verdict, &function.function_id, "function function_id");
        if !regions.contains(function.body_region_id.as_str()) {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                function.function_id.clone(),
                "function body_region_id does not reference a declared region",
            ));
        }
        if function.is_entrypoint {
            entrypoints += 1;
        }
        for parameter in &function.parameters {
            let Some(value) = values.get(parameter.value_id.as_str()) else {
                verdict.push(Diagnostic::new(
                    DiagnosticCode::SchemaViolation,
                    function.function_id.clone(),
                    format!("parameter '{}' has no declared value", parameter.value_id),
                ));
                continue;
            };
            if value.type_ref != parameter.type_ref
                || value.origin != ValueOrigin::Parameter
                || value.origin_id.as_deref() != Some(function.function_id.as_str())
            {
                verdict.push(Diagnostic::new(
                    DiagnosticCode::SchemaViolation,
                    parameter.value_id.clone(),
                    "function parameter does not match its typed parameter value origin",
                ));
            }
        }
    }
    if entrypoints != 1 {
        verdict.push(Diagnostic::new(
            DiagnosticCode::SchemaViolation,
            "functions",
            "a FrontendGraph declares exactly one entrypoint function",
        ));
    }

    for program in &graph.program_definitions {
        check_identifier(verdict, &program.program_id, "program program_id");
        if program.authoring.is_some()
            && program.has_default_context != program.default_context.is_some()
        {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                program.program_id.clone(),
                "authored Context default presence must match its carried literal",
            ));
        }
        if let Some(default) = &program.default_context {
            if program.context_type_ref.is_none() || !program.has_default_context {
                verdict.push(Diagnostic::new(
                    DiagnosticCode::SchemaViolation,
                    program.program_id.clone(),
                    "a default Context requires its declared Context type",
                ));
            }
            if let Err(reason) = default.literal_json() {
                verdict.push(Diagnostic::new(
                    DiagnosticCode::SchemaViolation,
                    program.program_id.clone(),
                    reason,
                ));
            }
        }
        if let Some(ProgramAuthoring::Agent { primary_model_ref }) = &program.authoring {
            check_identifier(verdict, primary_model_ref, "Agent primary_model_ref");
            let called = graph.call_intents.iter().any(|intent| {
                intent.intent_kind == IntentKind::ModelInvocation
                    && intent.binding_ref.as_deref().is_some_and(|binding| {
                        graph.declarations.iter().any(|declaration| {
                            declaration.decl_id == binding
                                && declaration.decl_kind == DeclKind::ModelBinding
                                && declaration.target_ref.as_ref() == Some(primary_model_ref)
                        })
                    })
            });
            let required = graph
                .model_requirements
                .iter()
                .any(|requirement| requirement.model_target_ref == *primary_model_ref);
            if !called || !required {
                verdict.push(Diagnostic::new(
                    DiagnosticCode::SchemaViolation,
                    program.program_id.clone(),
                    "an Agent primary model must have a captured model invocation and exact model requirement",
                ));
            }
        }
        if let Some(schema) = &program.input_schema
            && let Err(reason) = schema.validate()
        {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                program.program_id.clone(),
                reason,
            ));
        }
        if !functions.contains_key(program.entrypoint.as_str()) {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                program.program_id.clone(),
                "program entrypoint does not reference a declared function",
            ));
        }
    }

    for declaration in &graph.declarations {
        check_identifier(verdict, &declaration.decl_id, "declaration decl_id");
        check_identifier(
            verdict,
            &declaration.input_type_ref,
            "declaration input_type_ref",
        );
        check_identifier(
            verdict,
            &declaration.output_type_ref,
            "declaration output_type_ref",
        );
        if matches!(
            declaration.decl_kind,
            DeclKind::ModelBinding
                | DeclKind::ToolBinding
                | DeclKind::CapabilityBinding
                | DeclKind::EventType
        ) && declaration.target_ref.is_none()
        {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                declaration.decl_id.clone(),
                "external declaration is missing its exact target_ref",
            ));
        }
        // A `target_ref` is the name a Capability, model, or Event resolves by
        // downstream. Checking only its presence admits a reference no
        // catalogue lookup can ever match, so it carries the same identifier
        // grammar as every other reference in the graph.
        if let Some(target_ref) = &declaration.target_ref {
            check_identifier(verdict, target_ref, "declaration target_ref");
        }
    }

    for requirement in &graph.capability_requirements {
        check_identifier(
            verdict,
            &requirement.capability_ref,
            "capability requirement capability_ref",
        );
    }

    for value in &graph.values {
        let assembled = matches!(
            value.origin,
            ValueOrigin::Literal | ValueOrigin::ContextValue
        );
        if assembled != value.expression.is_some() {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                value.value_id.clone(),
                "literal and context values carry exactly one authored expression",
            ));
        }
        if let Some(expression) = &value.expression {
            let mut references = Vec::new();
            if !validate_value_expression(expression, &value.value_id, 0, &mut references) {
                verdict.push(Diagnostic::new(
                    DiagnosticCode::SchemaViolation,
                    value.value_id.clone(),
                    "authored value expression is malformed, unsafe, or exceeds bounds",
                ));
            }
            for dependency in references {
                if !values.contains_key(dependency.as_str()) {
                    verdict.push(Diagnostic::new(
                        DiagnosticCode::SchemaViolation,
                        value.value_id.clone(),
                        "authored value expression references an undeclared SSA value",
                    ));
                }
            }
        }
        match value.origin {
            ValueOrigin::Parameter => {
                if value
                    .origin_id
                    .as_deref()
                    .is_none_or(|function_id| !functions.contains_key(function_id))
                {
                    verdict.push(Diagnostic::new(
                        DiagnosticCode::SchemaViolation,
                        value.value_id.clone(),
                        "parameter value origin_id does not reference a function",
                    ));
                }
            }
            ValueOrigin::CallResult => {
                let valid = value
                    .origin_id
                    .as_deref()
                    .and_then(|node_id| call_intents.get(node_id))
                    .is_some_and(|intent| {
                        intent.result_value.as_deref() == Some(value.value_id.as_str())
                    });
                if !valid {
                    verdict.push(Diagnostic::new(
                        DiagnosticCode::SchemaViolation,
                        value.value_id.clone(),
                        "call-result value origin does not match a call intent result",
                    ));
                }
            }
            ValueOrigin::BlockArgument => {
                let valid = value
                    .origin_id
                    .as_deref()
                    .and_then(|block_id| blocks.get(block_id))
                    .is_some_and(|block| {
                        block.block_arguments.iter().any(|id| id == &value.value_id)
                    });
                if !valid {
                    verdict.push(Diagnostic::new(
                        DiagnosticCode::SchemaViolation,
                        value.value_id.clone(),
                        "block-argument value origin does not match a block argument",
                    ));
                }
            }
            ValueOrigin::ContextValue | ValueOrigin::Literal => {
                if value.origin_id.is_some() {
                    verdict.push(Diagnostic::new(
                        DiagnosticCode::SchemaViolation,
                        value.value_id.clone(),
                        "context and literal values do not carry an origin_id",
                    ));
                }
            }
            ValueOrigin::ResumeInput => {
                if value
                    .origin_id
                    .as_deref()
                    .and_then(|node_id| control_intents.get(node_id))
                    .is_none_or(|control| {
                        control.control_kind != ControlKind::Yield
                            || control.result_value.as_deref() != Some(value.value_id.as_str())
                    })
                {
                    verdict.push(Diagnostic::new(
                        DiagnosticCode::SchemaViolation,
                        value.value_id.clone(),
                        "resume-input value origin_id does not reference a control intent",
                    ));
                }
            }
        }
    }

    for block in &graph.blocks {
        let mut seen_args = HashSet::new();
        for value_id in &block.block_arguments {
            if !seen_args.insert(value_id.as_str()) {
                verdict.push(Diagnostic::new(
                    DiagnosticCode::SchemaViolation,
                    block.block_id.clone(),
                    "block_arguments contains the same value more than once",
                ));
            }
            if !values.contains_key(value_id.as_str()) {
                verdict.push(Diagnostic::new(
                    DiagnosticCode::SchemaViolation,
                    block.block_id.clone(),
                    format!("block argument '{value_id}' has no declared value"),
                ));
            }
        }
    }

    let mut edge_slots = HashSet::new();
    for edge in &graph.data_edges {
        if !values.contains_key(edge.from_value.as_str()) {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                edge.from_value.clone(),
                "data edge source does not reference a declared value",
            ));
        }
        if !call_intents.contains_key(edge.to_consumer.as_str())
            && !control_intents.contains_key(edge.to_consumer.as_str())
        {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                edge.to_consumer.clone(),
                "data edge consumer does not reference a call or control intent",
            ));
        }
        if !edge_slots.insert((edge.to_consumer.as_str(), edge.consumer_slot.as_str())) {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                edge.to_consumer.clone(),
                "data edges assign the same consumer slot more than once",
            ));
        }
    }

    for intent in &graph.call_intents {
        validate_call_intent(verdict, intent, &declarations, &values, graph);
    }
    for intent in &graph.control_intents {
        for region_id in &intent.body_region_ids {
            if !regions.contains(region_id.as_str()) {
                verdict.push(Diagnostic::new(
                    DiagnosticCode::SchemaViolation,
                    intent.node_id.clone(),
                    format!("control body region '{region_id}' is not declared"),
                ));
            }
        }
        if intent.control_kind == ControlKind::Loop && intent.body_region_ids.len() != 1 {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                intent.node_id.clone(),
                "a loop intent owns exactly one body region",
            ));
        }
        validate_control_predicate(verdict, intent, &values, graph);
    }
    let nodes = graph
        .call_intents
        .iter()
        .map(|intent| intent.node_id.as_str())
        .chain(
            graph
                .control_intents
                .iter()
                .map(|intent| intent.node_id.as_str()),
        )
        .collect::<HashSet<_>>();
    let endpoints = nodes
        .iter()
        .copied()
        .chain(graph.regions.iter().map(|region| region.region_id.as_str()))
        .collect::<HashSet<_>>();
    let region_parent: HashMap<&str, Option<&str>> = graph
        .regions
        .iter()
        .map(|region| {
            (
                region.region_id.as_str(),
                region.parent_region_id.as_deref(),
            )
        })
        .collect();
    let region_order: HashMap<&str, u32> = graph
        .regions
        .iter()
        .map(|region| (region.region_id.as_str(), region.execution_order))
        .collect();
    let function_bodies: HashMap<&str, &str> = graph
        .functions
        .iter()
        .map(|function| {
            (
                function.function_id.as_str(),
                function.body_region_id.as_str(),
            )
        })
        .collect();
    let ssa_context = SsaValidationContext {
        values: &values,
        blocks: &blocks,
        calls: &call_intents,
        controls: &control_intents,
        functions: &function_bodies,
        region_parent: &region_parent,
        region_order: &region_order,
    };
    for edge in &graph.context_flow {
        let valid_value = values.get(edge.value_id.as_str()).is_some_and(|value| {
            value.origin == ValueOrigin::ContextValue && value.type_ref == edge.context_type_ref
        });
        let ordered = context_endpoint_location(
            &edge.from_node,
            &call_intents,
            &control_intents,
            &region_definitions,
        )
        .zip(context_endpoint_location(
            &edge.to_node,
            &call_intents,
            &control_intents,
            &region_definitions,
        ))
        .is_some_and(|(from, to)| {
            (from.region_id != to.region_id
                || from.execution_order != to.execution_order
                || from.entry != to.entry)
                && dominates_location(from, to, &ssa_context)
        });
        if !endpoints.contains(edge.from_node.as_str())
            || !endpoints.contains(edge.to_node.as_str())
            || !valid_value
            || !ordered
        {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                edge.to_node.clone(),
                "Context edge requires a typed context value, valid context coordinate, and source-before-destination endpoints",
            ));
        }
    }
    validate_ssa_dominance(verdict, graph, &values);
}

#[derive(Clone)]
struct SsaLocation {
    region_id: String,
    execution_order: u32,
    entry: bool,
}

struct SsaValidationContext<'a> {
    values: &'a HashMap<&'a str, &'a Value>,
    blocks: &'a HashMap<&'a str, &'a Block>,
    calls: &'a HashMap<&'a str, &'a CallIntent>,
    controls: &'a HashMap<&'a str, &'a ControlIntent>,
    functions: &'a HashMap<&'a str, &'a str>,
    region_parent: &'a HashMap<&'a str, Option<&'a str>>,
    region_order: &'a HashMap<&'a str, u32>,
}

/// Ensure every authored SSA dependency is available at every consumer. A
/// declaration existing somewhere in the graph is insufficient: future
/// results and sibling-branch values must never become executable invocation
/// operands. A completed nested region may export its result to a later use in
/// the containing region.
fn validate_ssa_dominance(
    verdict: &mut Verdict,
    graph: &FrontendGraph,
    values: &HashMap<&str, &Value>,
) {
    validate_value_expression_cycles(verdict, values);
    let region_parent: HashMap<&str, Option<&str>> = graph
        .regions
        .iter()
        .map(|region| {
            (
                region.region_id.as_str(),
                region.parent_region_id.as_deref(),
            )
        })
        .collect();
    let region_order: HashMap<&str, u32> = graph
        .regions
        .iter()
        .map(|region| (region.region_id.as_str(), region.execution_order))
        .collect();
    let functions: HashMap<&str, &str> = graph
        .functions
        .iter()
        .map(|function| {
            (
                function.function_id.as_str(),
                function.body_region_id.as_str(),
            )
        })
        .collect();
    let blocks: HashMap<&str, &Block> = graph
        .blocks
        .iter()
        .map(|block| (block.block_id.as_str(), block))
        .collect();
    let calls: HashMap<&str, &CallIntent> = graph
        .call_intents
        .iter()
        .map(|intent| (intent.node_id.as_str(), intent))
        .collect();
    let controls: HashMap<&str, &ControlIntent> = graph
        .control_intents
        .iter()
        .map(|intent| (intent.node_id.as_str(), intent))
        .collect();
    let context = SsaValidationContext {
        values,
        blocks: &blocks,
        calls: &calls,
        controls: &controls,
        functions: &functions,
        region_parent: &region_parent,
        region_order: &region_order,
    };

    let check_use = |value_id: &str, consumer: &str| {
        let Some(location) = consumer_location(consumer, &calls, &controls) else {
            return true;
        };
        let mut visiting = HashSet::new();
        value_dominates_use(value_id, location, &context, &mut visiting)
    };

    for intent in graph
        .call_intents
        .iter()
        .filter(|intent| is_executable_invocation(intent.intent_kind))
    {
        // Check the authored operand list itself. Data edges still provide
        // typed slots for lowering, but dominance must not disappear merely
        // because a malformed graph omitted one of those edges.
        for value_id in &intent.operand_values {
            if !check_use(value_id, &intent.node_id) {
                verdict.push(Diagnostic::new(
                    DiagnosticCode::SchemaViolation,
                    intent.node_id.clone(),
                    format!("SSA value '{value_id}' does not dominate its consumer"),
                ));
            }
        }
    }
}

fn is_executable_invocation(kind: IntentKind) -> bool {
    matches!(
        kind,
        IntentKind::ModelInvocation
            | IntentKind::ToolInvocation
            | IntentKind::CapabilityInvocation
            | IntentKind::AgentInvocation
            | IntentKind::AgentCreation
            | IntentKind::EventWait
    )
}

fn validate_value_expression_cycles(verdict: &mut Verdict, values: &HashMap<&str, &Value>) {
    let mut state = HashMap::<&str, u8>::new();
    for value_id in values.keys().copied() {
        if value_expression_cycle(value_id, values, &mut state) {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                value_id.to_string(),
                "authored value expressions contain a cycle",
            ));
        }
    }
}

fn value_expression_cycle<'a>(
    value_id: &'a str,
    values: &HashMap<&'a str, &'a Value>,
    state: &mut HashMap<&'a str, u8>,
) -> bool {
    match state.get(value_id).copied() {
        Some(1) => return true,
        Some(2) => return false,
        _ => {}
    }
    state.insert(value_id, 1);
    let cycle = values
        .get(value_id)
        .and_then(|value| value.expression.as_ref())
        .is_some_and(|expression| {
            let mut references = Vec::new();
            collect_expression_references(expression, &mut references);
            references.into_iter().any(|reference| {
                values
                    .keys()
                    .copied()
                    .find(|key| *key == reference)
                    .is_some_and(|key| value_expression_cycle(key, values, state))
            })
        });
    state.insert(value_id, 2);
    cycle
}

fn consumer_location(
    consumer: &str,
    calls: &HashMap<&str, &CallIntent>,
    controls: &HashMap<&str, &ControlIntent>,
) -> Option<SsaLocation> {
    calls
        .get(consumer)
        .map(|intent| SsaLocation {
            region_id: intent.parent_region_id.clone(),
            execution_order: intent.execution_order,
            entry: false,
        })
        .or_else(|| {
            controls.get(consumer).map(|intent| SsaLocation {
                region_id: intent.parent_region_id.clone(),
                execution_order: intent.execution_order,
                entry: false,
            })
        })
}

fn context_endpoint_location(
    endpoint: &str,
    calls: &HashMap<&str, &CallIntent>,
    controls: &HashMap<&str, &ControlIntent>,
    regions: &HashMap<&str, &Region>,
) -> Option<SsaLocation> {
    consumer_location(endpoint, calls, controls).or_else(|| {
        regions.get(endpoint).map(|region| SsaLocation {
            region_id: region.region_id.clone(),
            execution_order: 0,
            entry: true,
        })
    })
}

fn region_is_terminal(region_id: &str, context: &SsaValidationContext<'_>) -> bool {
    context.controls.values().any(|control| {
        control.parent_region_id == region_id
            && matches!(
                control.control_kind,
                ControlKind::Throw | ControlKind::Return | ControlKind::Yield
            )
    })
}

fn value_dominates_use(
    value_id: &str,
    use_location: SsaLocation,
    context: &SsaValidationContext<'_>,
    visiting: &mut HashSet<String>,
) -> bool {
    if !visiting.insert(value_id.to_string()) {
        return false;
    }
    let Some(value) = context.values.get(value_id) else {
        return false;
    };
    let dominates = match value.origin {
        ValueOrigin::CallResult => value
            .origin_id
            .as_deref()
            .and_then(|node_id| context.calls.get(node_id))
            .is_some_and(|intent| {
                dominates_location(
                    SsaLocation {
                        region_id: intent.parent_region_id.clone(),
                        execution_order: intent.execution_order,
                        entry: false,
                    },
                    use_location.clone(),
                    context,
                )
            }),
        ValueOrigin::BlockArgument => value
            .origin_id
            .as_deref()
            .and_then(|block_id| context.blocks.get(block_id))
            .is_some_and(|block| {
                dominates_location(
                    SsaLocation {
                        region_id: block.region_id.clone(),
                        execution_order: 0,
                        entry: true,
                    },
                    use_location.clone(),
                    context,
                )
            }),
        ValueOrigin::Parameter => value
            .origin_id
            .as_deref()
            .and_then(|function_id| context.functions.get(function_id))
            .is_some_and(|region_id| {
                dominates_location(
                    SsaLocation {
                        region_id: (*region_id).to_string(),
                        execution_order: 0,
                        entry: true,
                    },
                    use_location.clone(),
                    context,
                )
            }),
        // Resume values are produced by the structural yield and consumed by
        // its loop-back edge, which is not an ordinary forward SSA use.
        ValueOrigin::ResumeInput | ValueOrigin::ContextValue | ValueOrigin::Literal => true,
    };
    if !dominates {
        return false;
    }

    let expression_ok = match value.expression.as_ref() {
        Some(expression) => expression_references(expression)
            .into_iter()
            .all(|dependency| {
                value_dominates_use(&dependency, use_location.clone(), context, visiting)
            }),
        None => true,
    };
    visiting.remove(value_id);
    expression_ok
}

fn expression_references(expression: &ValueExpression) -> Vec<String> {
    let mut references = Vec::new();
    collect_expression_references(expression, &mut references);
    references
}

fn collect_expression_references(expression: &ValueExpression, out: &mut Vec<String>) {
    match expression {
        ValueExpression::Ssa { value_id } => out.push(value_id.clone()),
        ValueExpression::Projection { root, .. } => collect_expression_references(root, out),
        ValueExpression::Object { fields } => fields
            .iter()
            .for_each(|field| collect_expression_references(&field.value, out)),
        ValueExpression::Array { items } => items
            .iter()
            .for_each(|item| collect_expression_references(item, out)),
        ValueExpression::Context { .. }
        | ValueExpression::String { .. }
        | ValueExpression::Integer { .. }
        | ValueExpression::Boolean { .. }
        | ValueExpression::Null => {}
    }
}

fn dominates_location(
    definition: SsaLocation,
    use_location: SsaLocation,
    context: &SsaValidationContext<'_>,
) -> bool {
    if definition.region_id == use_location.region_id {
        return definition.entry || definition.execution_order < use_location.execution_order;
    }
    if is_ancestor(
        &definition.region_id,
        &use_location.region_id,
        context.region_parent,
    ) {
        let mut child = use_location.region_id.as_str();
        while let Some(parent) = context.region_parent.get(child).copied().flatten() {
            if parent == definition.region_id {
                return definition.entry
                    || context
                        .region_order
                        .get(child)
                        .is_some_and(|child_order| definition.execution_order < *child_order);
            }
            if context.controls.values().any(|control| {
                control.parent_region_id == parent
                    && control
                        .body_region_ids
                        .iter()
                        .any(|region_id| region_id == child)
                    && control.body_region_ids.len() > 1
                    && control
                        .body_region_ids
                        .iter()
                        .filter(|region_id| region_id.as_str() != child)
                        .any(|region_id| !region_is_terminal(region_id, context))
            }) {
                return false;
            }
            child = parent;
        }
        return definition.entry;
    }

    // A nested invocation result is available after its containing structural
    // region completes. This is the only descendant-to-ancestor case allowed:
    // sibling regions remain incomparable and therefore fail closed.
    if is_ancestor(
        &use_location.region_id,
        &definition.region_id,
        context.region_parent,
    ) {
        let mut child = definition.region_id.as_str();
        while let Some(parent) = context.region_parent.get(child).copied().flatten() {
            if parent == use_location.region_id {
                return context
                    .region_order
                    .get(child)
                    .is_some_and(|child_order| *child_order < use_location.execution_order);
            }
            child = parent;
        }
    }
    false
}

fn is_ancestor(
    ancestor: &str,
    descendant: &str,
    region_parent: &HashMap<&str, Option<&str>>,
) -> bool {
    let mut cursor = Some(descendant);
    while let Some(region) = cursor {
        if region == ancestor {
            return true;
        }
        cursor = region_parent.get(region).copied().flatten();
    }
    false
}

fn validate_control_predicate(
    verdict: &mut Verdict,
    intent: &ControlIntent,
    values: &HashMap<&str, &Value>,
    graph: &FrontendGraph,
) {
    let Some(predicate) = &intent.predicate else {
        if matches!(
            intent.control_kind,
            ControlKind::Conditional | ControlKind::Switch
        ) {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                intent.node_id.clone(),
                "conditional and switch intents require a typed predicate",
            ));
        }
        return;
    };
    if !matches!(
        intent.control_kind,
        ControlKind::Conditional | ControlKind::Switch | ControlKind::Loop
    ) {
        verdict.push(Diagnostic::new(
            DiagnosticCode::SchemaViolation,
            intent.node_id.clone(),
            "only conditional, switch, and loop intents carry predicates",
        ));
    }
    if !values.contains_key(predicate.root_value_id.as_str()) {
        verdict.push(Diagnostic::new(
            DiagnosticCode::SchemaViolation,
            intent.node_id.clone(),
            "predicate root_value_id does not reference a declared value",
        ));
    }
    if predicate.property_path.len() > 16
        || predicate.property_path.iter().any(|segment| {
            segment.is_empty()
                || segment.len() > 128
                || !segment
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        })
    {
        verdict.push(Diagnostic::new(
            DiagnosticCode::SchemaViolation,
            intent.node_id.clone(),
            "predicate property_path has an empty, hostile, or overlong segment",
        ));
    }
    match (predicate.comparator, predicate.literal.as_ref()) {
        (PredicateComparator::Truthy, None)
        | (PredicateComparator::Equals | PredicateComparator::NotEquals, Some(_)) => {}
        (PredicateComparator::Truthy, Some(_)) => verdict.push(Diagnostic::new(
            DiagnosticCode::SchemaViolation,
            intent.node_id.clone(),
            "truthy predicate does not accept a literal",
        )),
        (PredicateComparator::Equals | PredicateComparator::NotEquals, None) => {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                intent.node_id.clone(),
                "equals predicate requires a typed scalar literal",
            ));
        }
    }
    // Both source frontends commit this schema. A loop-carried input retains
    // its declared type, so the same check covers initial and later messages
    // without language-specific truthiness or a runtime coercion rule.
    if predicate.comparator == PredicateComparator::Truthy
        && let [program] = graph.program_definitions.as_slice()
        && let Some(value) = values.get(predicate.root_value_id.as_str())
        && value.type_ref == program.input_type_ref
        && let Some(schema) = &program.input_schema
    {
        let projected = predicate
            .property_path
            .iter()
            .try_fold(schema, |schema, property| {
                schema.properties.as_ref()?.get(property)
            });
        if projected
            .is_some_and(|schema| schema.kind != crate::input_schema::InputSchemaType::Boolean)
        {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                intent.node_id.clone(),
                "truthy predicate requires a boolean typed input; compare nonboolean values explicitly",
            ));
        }
    }
    if matches!(predicate.literal, Some(PredicateLiteral::Integer(value)) if value.unsigned_abs() > MAX_SAFE_PREDICATE_INTEGER)
    {
        verdict.push(Diagnostic::new(
            DiagnosticCode::SchemaViolation,
            intent.node_id.clone(),
            "predicate integer literal exceeds the shared safe integer domain",
        ));
    }
}

fn valid_expression_path(path: &[String], empty_allowed: bool) -> bool {
    (empty_allowed || !path.is_empty())
        && path.len() <= 16
        && path.iter().all(|segment| {
            !segment.is_empty()
                && segment.len() <= 128
                && segment
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        })
}

fn validate_value_expression(
    expression: &ValueExpression,
    owner: &str,
    depth: usize,
    references: &mut Vec<String>,
) -> bool {
    if depth > 64 {
        return false;
    }
    match expression {
        ValueExpression::Ssa { value_id } => {
            if !is_identifier(value_id) || value_id == owner {
                return false;
            }
            references.push(value_id.clone());
            true
        }
        ValueExpression::Context { property_path } => valid_expression_path(property_path, true),
        ValueExpression::Projection {
            root,
            property_path,
        } => {
            valid_expression_path(property_path, false)
                && validate_value_expression(root, owner, depth + 1, references)
        }
        ValueExpression::Object { fields } => {
            let mut names = HashSet::new();
            fields.len() <= 64
                && fields.iter().all(|field| {
                    !field.name.is_empty()
                        && field.name.len() <= 128
                        && field
                            .name
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
                        && names.insert(field.name.as_str())
                        && validate_value_expression(&field.value, owner, depth + 1, references)
                })
        }
        ValueExpression::Array { items } => {
            items.len() <= 256
                && items
                    .iter()
                    .all(|item| validate_value_expression(item, owner, depth + 1, references))
        }
        ValueExpression::String { value } => value.len() <= 1_048_576,
        ValueExpression::Integer { value } => value.unsigned_abs() <= MAX_SAFE_PREDICATE_INTEGER,
        ValueExpression::Boolean { .. } | ValueExpression::Null => true,
    }
}

fn check_unique_ids<'a>(verdict: &mut Verdict, ids: impl Iterator<Item = &'a str>, kind: &str) {
    let mut seen = HashSet::new();
    for id in ids {
        if !seen.insert(id) {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                id,
                format!("{kind} id is not unique"),
            ));
        }
    }
}

fn validate_call_intent(
    verdict: &mut Verdict,
    intent: &CallIntent,
    declarations: &HashMap<&str, &Declaration>,
    values: &HashMap<&str, &Value>,
    graph: &FrontendGraph,
) {
    let declaration_matches = |kind: DeclKind| match intent.intent_kind {
        IntentKind::ModelInvocation => kind == DeclKind::ModelBinding,
        IntentKind::ToolInvocation => kind == DeclKind::ToolBinding,
        IntentKind::CapabilityInvocation => kind == DeclKind::CapabilityBinding,
        IntentKind::EventWait => kind == DeclKind::EventType,
        IntentKind::AgentCreation | IntentKind::AgentInvocation => false,
    };
    if matches!(
        intent.intent_kind,
        IntentKind::ModelInvocation | IntentKind::ToolInvocation | IntentKind::CapabilityInvocation
    ) {
        let declaration = intent
            .binding_ref
            .as_deref()
            .and_then(|id| declarations.get(id));
        match declaration {
            Some(declaration) if declaration_matches(declaration.decl_kind) => {}
            _ => {
                verdict.push(Diagnostic::new(
                    DiagnosticCode::SchemaViolation,
                    intent.node_id.clone(),
                    "call intent binding_ref does not resolve to the required typed declaration",
                ));
            }
        }
        let target_ref = declaration.and_then(|declaration| declaration.target_ref.as_deref());
        let requirement_present = match intent.intent_kind {
            IntentKind::ModelInvocation => target_ref.is_some_and(|target| {
                graph
                    .model_requirements
                    .iter()
                    .any(|requirement| requirement.model_target_ref == target)
            }),
            IntentKind::ToolInvocation | IntentKind::CapabilityInvocation => target_ref
                .is_some_and(|target| {
                    graph
                        .capability_requirements
                        .iter()
                        .any(|requirement| requirement.capability_ref == target)
                }),
            _ => true,
        };
        if !requirement_present {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                intent.node_id.clone(),
                "authored effect binding is missing its exact declared requirement",
            ));
        }
    } else if intent.intent_kind == IntentKind::EventWait {
        if let Some(binding_ref) = intent.binding_ref.as_deref()
            && !declarations
                .get(binding_ref)
                .is_some_and(|declaration| declaration_matches(declaration.decl_kind))
        {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                intent.node_id.clone(),
                "event binding_ref does not resolve to a typed Event declaration",
            ));
        }
    } else if let Some(binding_ref) = intent.binding_ref.as_deref() {
        let known_program = graph
            .program_definitions
            .iter()
            .any(|program| program.program_id == binding_ref)
            || graph
                .imported_program_refs
                .iter()
                .any(|program| program.program_ref == binding_ref);
        if !known_program {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                intent.node_id.clone(),
                "program call binding_ref does not resolve to a local or imported ProgramRef",
            ));
        }
    } else {
        verdict.push(Diagnostic::new(
            DiagnosticCode::SchemaViolation,
            intent.node_id.clone(),
            "call intent is missing its typed binding_ref",
        ));
    }

    let Some(result_value) = intent.result_value.as_deref() else {
        verdict.push(Diagnostic::new(
            DiagnosticCode::SchemaViolation,
            intent.node_id.clone(),
            "effect intent is missing its typed result value",
        ));
        return;
    };
    let valid_result = values.get(result_value).is_some_and(|value| {
        value.origin == ValueOrigin::CallResult
            && value.origin_id.as_deref() == Some(intent.node_id.as_str())
    });
    if !valid_result {
        verdict.push(Diagnostic::new(
            DiagnosticCode::SchemaViolation,
            intent.node_id.clone(),
            "effect intent result_value does not resolve to its call-result value",
        ));
    }
}

/// Verify that every Hook binding is backed by the captured body region it
/// names, and that its return mode is the one that body actually implies.
///
/// Without this a binding and the structure it claims to describe are two
/// independent copies: the fields would still decode, the AIR would still
/// lower, and a Hook declared `observe` could carry a body that rewrites
/// Context. Every `hook_body` region belongs to exactly one binding, so an
/// unclaimed captured body cannot smuggle effects into a program either.
fn collect_hook_diagnostics(verdict: &mut Verdict, graph: &FrontendGraph) {
    let mut claimed_bodies: HashSet<&str> = HashSet::new();
    let mut declared_ids: HashSet<&str> = HashSet::new();
    for hook in &graph.hook_bindings {
        check_identifier(verdict, &hook.hook_id, "hook_id");
        // `hook_id` is how every downstream consumer names one Hook — evidence,
        // artifact/AIR reconciliation, and handler resolution all key on it — so
        // two bindings sharing one id make those lookups answer for whichever
        // binding happens to be found first.
        if !declared_ids.insert(hook.hook_id.as_str()) {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                hook.hook_id.clone(),
                "two Hook bindings declare the same hook_id",
            ));
        }
        check_identifier(verdict, &hook.handler_ref, "hook handler_ref");
        check_digest(verdict, &hook.handler_digest, &hook.hook_id);
        check_identifier(verdict, &hook.body_region_id, "hook body_region_id");

        let body = graph
            .regions
            .iter()
            .find(|region| region.region_id == hook.body_region_id);
        match body {
            Some(region) if region.region_role == RegionRole::HookBody => {}
            Some(_) => verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                hook.hook_id.clone(),
                "hook body_region_id names a region that is not a captured Hook body",
            )),
            None => verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                hook.hook_id.clone(),
                "hook body_region_id does not reference a declared region",
            )),
        }
        if !claimed_bodies.insert(hook.body_region_id.as_str()) {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                hook.hook_id.clone(),
                "two Hook bindings claim the same captured body region",
            ));
        }

        let assigned = hook.assigned_context_value_id.as_deref();
        match (hook.return_mode, assigned) {
            (HookReturnMode::Observe, None) => {}
            (HookReturnMode::ReplaceResult, Some(value_id)) => {
                let assigns_context = graph.values.iter().any(|value| {
                    value.value_id == value_id && value.origin == ValueOrigin::ContextValue
                });
                if !assigns_context {
                    verdict.push(Diagnostic::new(
                        DiagnosticCode::SchemaViolation,
                        hook.hook_id.clone(),
                        "hook assigned_context_value_id does not reference a typed Context value",
                    ));
                }
            }
            (HookReturnMode::Observe, Some(_)) => verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                hook.hook_id.clone(),
                "an observing Hook assigns no Context value",
            )),
            (HookReturnMode::ReplaceResult, None) => verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                hook.hook_id.clone(),
                "a replacing Hook names the Context value its captured body assigns",
            )),
        }
    }

    for region in graph
        .regions
        .iter()
        .filter(|region| region.region_role == RegionRole::HookBody)
    {
        if !claimed_bodies.contains(region.region_id.as_str()) {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                region.region_id.clone(),
                "captured Hook body region is claimed by no Hook binding",
            ));
        }
    }
}

/// Verify every authored skill declaration.
///
/// Decode already closed the instruction branches, so what is left is what
/// decode cannot see: that a skill is identified once, that a file-carried
/// skill names the one path its id resolves to rather than an independent
/// second spelling of it, and that an inline body is neither empty nor larger
/// than the reader will load.
fn collect_skill_diagnostics(verdict: &mut Verdict, graph: &FrontendGraph) {
    let mut seen: HashSet<&str> = HashSet::new();
    for requirement in &graph.skill_requirements {
        check_identifier(verdict, &requirement.skill_id, "skill requirement skill_id");
        if !seen.insert(requirement.skill_id.as_str()) {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                requirement.skill_id.clone(),
                "skill requirement skill_id is not unique",
            ));
        }
        match &requirement.instruction_source {
            SkillInstructionSource::Entry { path } => {
                if *path != SkillRequirement::entry_path_for(&requirement.skill_id) {
                    verdict.push(Diagnostic::new(
                        DiagnosticCode::SchemaViolation,
                        requirement.skill_id.clone(),
                        "a file-carried skill's instruction path is skills/<skill_id>/SKILL.md",
                    ));
                }
            }
            SkillInstructionSource::Inline { text } => {
                if text.is_empty() || text.len() > MAX_INSTRUCTION_BYTES {
                    verdict.push(Diagnostic::new(
                        DiagnosticCode::SchemaViolation,
                        requirement.skill_id.clone(),
                        "an inline skill body is neither empty nor beyond the instruction ceiling",
                    ));
                }
            }
        }
    }
}

fn collect_containment_diagnostics(verdict: &mut Verdict, graph: &FrontendGraph) {
    let regions: HashSet<&str> = graph
        .regions
        .iter()
        .map(|region| region.region_id.as_str())
        .collect();
    let mut positions: HashSet<(Option<&str>, u32)> = HashSet::new();
    let mut region_at_position: HashMap<(Option<&str>, u32), &str> = HashMap::new();

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
        let position = (region.parent_region_id.as_deref(), region.execution_order);
        if !positions.insert(position) {
            verdict.push(Diagnostic::new(
                DiagnosticCode::SchemaViolation,
                region.region_id.clone(),
                "sibling regions have duplicate execution_order",
            ));
        }
        region_at_position.insert(position, region.region_id.as_str());

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
        let position = (
            Some(intent.parent_region_id.as_str()),
            intent.execution_order,
        );
        let loop_replaces_its_body_region = intent.control_kind == ControlKind::Loop
            && region_at_position
                .get(&position)
                .is_some_and(|region_id| intent.body_region_ids.iter().any(|id| id == region_id));
        if !loop_replaces_its_body_region && !positions.insert(position) {
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
