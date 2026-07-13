//! AIS Passes - Single Source of Truth
//!
//! This module contains the complete specification for all AIS compiler passes.
//! Both the compiler and the C API layer use these definitions to ensure
//! consistent pass registration across the system.
//!
//! ## Generated Files
//!
//! The `tablegen` submodule generates the following files:
//! - `Passes.generated.td` - MLIR TableGen pass definitions
//! - `PassDispatch.inc` - C API dispatch switch statement
//! - `PassDescriptors.inc` - Pass registry descriptors

mod tablegen;

pub use tablegen::{generate_pass_descriptors, generate_pass_dispatch, generate_passes_tablegen};

// ============================================================================
// Pass Categories
// ============================================================================

/// Categories of compiler passes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PassCategory {
    /// Domain-specific transformations (normalize, fuse-ask-ops, scheduling).
    Transform,
    /// Standard MLIR optimizations (canonicalizer, cse, symbol-dce).
    Optimization,
    /// Analysis/warning passes (unconsumed-value-warning).
    Analysis,
    /// Dialect lowering passes (lower-to-async, ais-emit-rust).
    Lowering,
}

impl PassCategory {
    /// Get the C enum value for this category.
    pub fn to_c_enum(&self) -> &'static str {
        match self {
            PassCategory::Transform => "APXM_PASS_TRANSFORM",
            PassCategory::Optimization => "APXM_PASS_OPTIMIZATION",
            PassCategory::Analysis => "APXM_PASS_ANALYSIS",
            PassCategory::Lowering => "APXM_PASS_LOWERING",
        }
    }
}

// ============================================================================
// Pass Options
// ============================================================================

/// A pass option that can be configured.
#[derive(Debug, Clone)]
pub struct PassOption {
    /// Option name (e.g., "parallel-threshold").
    pub name: &'static str,
    /// C++ variable name (e.g., "parallelThreshold").
    pub cpp_name: &'static str,
    /// Option type (e.g., "unsigned", "std::string").
    pub option_type: &'static str,
    /// Default value.
    pub default: &'static str,
    /// Description.
    pub description: &'static str,
}

impl PassOption {
    pub const fn new(
        name: &'static str,
        cpp_name: &'static str,
        option_type: &'static str,
        default: &'static str,
        description: &'static str,
    ) -> Self {
        Self {
            name,
            cpp_name,
            option_type,
            default,
            description,
        }
    }
}

// ============================================================================
// Pass Specification
// ============================================================================

/// Complete specification for a compiler pass.
#[derive(Debug, Clone)]
pub struct PassSpec {
    /// CLI name (e.g., "fuse-ask-ops").
    pub name: &'static str,
    /// TableGen class name (e.g., "FuseAskOps").
    pub class_name: &'static str,
    /// Short summary for help text.
    pub summary: &'static str,
    /// Long description for documentation.
    pub description: &'static str,
    /// Pass category.
    pub category: PassCategory,
    /// C++ constructor call.
    pub constructor: &'static str,
    /// Dependent dialects (for lowering passes).
    pub dependent_dialects: &'static [&'static str],
    /// Configurable options.
    pub options: &'static [PassOption],
    /// Whether this is a built-in MLIR pass (not generated).
    pub is_builtin: bool,
}

impl PassSpec {
    /// Create a new pass specification with minimal required fields.
    pub const fn new(
        name: &'static str,
        class_name: &'static str,
        summary: &'static str,
        description: &'static str,
        category: PassCategory,
        constructor: &'static str,
    ) -> Self {
        Self {
            name,
            class_name,
            summary,
            description,
            category,
            constructor,
            dependent_dialects: &[],
            options: &[],
            is_builtin: false,
        }
    }

    /// Builder method to add dependent dialects.
    pub const fn with_dialects(mut self, dialects: &'static [&'static str]) -> Self {
        self.dependent_dialects = dialects;
        self
    }

    /// Builder method to add options.
    pub const fn with_options(mut self, options: &'static [PassOption]) -> Self {
        self.options = options;
        self
    }

    /// Mark this as a built-in MLIR pass.
    pub const fn builtin(mut self) -> Self {
        self.is_builtin = true;
        self
    }
}

// ============================================================================
// Pass Definitions - Single Source of Truth
// ============================================================================

/// Normalize pass - canonicalizes AIS graph structure.
pub const NORMALIZE: PassSpec = PassSpec::new(
    "normalize",
    "NormalizeAgentGraph",
    "Normalize AIS graph structure",
    r"Canonicalizes the AIS graph by:
- Deduplicating unnamed LLM context operands
- Normalizing string attributes (lowercase capability/memory-tier names)
- Establishing SSA ordering invariants for downstream passes

This pass ensures the IR is in a canonical form that other passes
can rely on, similar to MLIR's canonicalizer but domain-specific.",
    PassCategory::Transform,
    "mlir::ais::createNormalizeAgentGraphPass()",
);

/// BuildPrompt pass - materializes LLM prompt/input_names runtime contracts.
pub const BUILD_PROMPT: PassSpec = PassSpec::new(
    "build-prompt",
    "BuildPrompt",
    "Build LLM operation prompt templates",
    r#"Processes LLM operations and generates placeholder templates:
1. Empty template_str -> synthesized "{<name>}" templates referencing
   each context operand by its `input_names` entry.
2. Missing or malformed `input_names` -> synthesized `ctx0`, `ctx1`, ...
   entries that match the context operand arity.
3. Optionally embed instruction prompts from config.

Works with InstructionConfig system - does NOT replace runtime mapping,
just ensures LLM ops with context have a complete template/input_names
contract.

This pass enables proper prompt construction for operations like:
  ask(user_input) -> ask("{user_input}", [user_input])
  with the matching `input_names = ["user_input"]` attribute.

Without this pass, empty template_str or missing input_names causes the runtime
to produce empty prompts or reject context-bearing LLM nodes."#,
    PassCategory::Transform,
    "mlir::ais::createBuildPromptPass()",
)
.with_options(&[
    PassOption::new(
        "generate-placeholders",
        "generatePlaceholders",
        "bool",
        "true",
        "Generate named {<input>} placeholders when template_str is empty",
    ),
    PassOption::new(
        "embed-instructions",
        "embedInstructions",
        "bool",
        "false",
        "Embed instruction prompts into artifact (bypasses runtime config)",
    ),
]);

/// Scheduling pass - annotates operations with scheduling metadata.
pub const SCHEDULING: PassSpec = PassSpec::new(
    "scheduling",
    "CapabilityScheduling",
    "Annotate operations with scheduling metadata",
    r"Classifies capabilities into execution tiers and annotates operations
with scheduling hints for the runtime:

- Tier classification (io/compute/reasoning/memory/general)
- Cost estimation based on context size
- Parallel-safety markers for speculation

These annotations guide the runtime dataflow scheduler to overlap work
and optimize execution order.",
    PassCategory::Transform,
    "mlir::ais::createCapabilitySchedulingPass()",
)
.with_options(&[
    PassOption::new(
        "parallel-threshold",
        "parallelThreshold",
        "unsigned",
        "3",
        "Maximum context size considered safe for parallel execution",
    ),
    PassOption::new(
        "base-cost",
        "baseCost",
        "unsigned",
        "2",
        "Base estimated cost for each operation",
    ),
    PassOption::new(
        "context-weight",
        "contextWeight",
        "unsigned",
        "2",
        "Cost multiplier per context operand",
    ),
]);

/// Assign execution priority based on critical path analysis.
pub const ASSIGN_PRIORITY: PassSpec = PassSpec::new(
    "assign-priority",
    "AssignPriority",
    "Assign execution priority based on critical path analysis",
    r"Performs DAG analysis to compute the critical path and assigns priority
attributes to each AIS operation for the runtime scheduler.

Priority levels:
- 90 (Critical): Operations on or near the critical path
- 70 (High): Operations with high fan-out (3+ consumers)
- 30 (Normal): All other operations

The priority attribute is stored as an IntegerAttr and later extracted by
the ArtifactEmitter into node.metadata.priority, which the runtime scheduler
uses to assign work to the 4-level priority queue (Critical/High/Normal/Low).",
    PassCategory::Transform,
    "mlir::ais::createAssignPriorityPass()",
);

/// SharedPrefixAnalysis pass - annotate existing shared-prefix opportunities.
pub const SHARED_PREFIX_ANALYSIS: PassSpec = PassSpec::new(
    "shared-prefix-analysis",
    "SharedPrefixAnalysis",
    "Annotate existing shared-prefix reuse opportunities",
    r"Detects LLM operations that already share the same leading prompt prefix
and context operands, then emits graph-hint metadata for prefix-aware backends.

This pass does not rewrite prompt templates or reorder operands. It only emits
analysis metadata such as shared-prefix group, estimated shared-prefix tokens,
and warmup candidate markers.",
    PassCategory::Transform,
    "mlir::ais::createSharedPrefixAnalysisPass()",
);

/// Fuse ask ops pass - merges adjacent ask operations.
pub const FUSE_ASK_OPS: PassSpec = PassSpec::new(
    "fuse-ask-ops",
    "FuseAskOps",
    "Explicit-only ASK fusion experiment",
    r"Identifies producer-consumer ais.ask chains and merges them into single
batched operations:

- Reduces serialized LLM API calls
- Concatenates ask templates with separator
- Combines contexts from both operations

Only fuses AskOp (LOW latency). ThinkOp/ReasonOp are not fused because
they have different semantics (extended thinking, structured output).

This pass is explicit-only until APXM has typed request-attribute preservation
and semantic-quality heuristics for LLM-call merging. Only fuses when producer
has single use.",
    PassCategory::Transform,
    "mlir::ais::createFuseAskOpsPass()",
);

/// CondenseOps pass - condenses consecutive memory operations.
pub const CONDENSE_OPS: PassSpec = PassSpec::new(
    "condense-ops",
    "CondenseOps",
    "Explicit-only memory operation batching experiment",
    r"Identifies linear chains of QMEM or UMEM operations that target the same
memory space and condenses them into a single batched operation.

For QMEM chains, queries are concatenated (newline-separated) into a single
query, reducing memory round-trips.  For UMEM chains, redundant intermediate
writes to the same space are eliminated (last-write-wins).

Condensation fires when:
1. Two or more consecutive QMEM/UMEM ops target the same memory space
2. They share the same stage (sid) for QMEM
3. No intervening side-effectful operations exist between them
4. Intermediate results are not consumed by other operations

This pass is explicit-only until memory-store semantics and batching capability
contracts are typed.",
    PassCategory::Transform,
    "mlir::ais::createCondenseOpsPass()",
);

/// Unconsumed value warning pass - analysis pass for detecting unused results.
pub const UNCONSUMED_VALUE_WARNING: PassSpec = PassSpec::new(
    "unconsumed-value-warning",
    "UnconsumedValueWarning",
    "Warn about unconsumed operation results",
    r#"Scans all operations and emits warnings when operation results are not
consumed by any other operation. This helps developers identify:

- Forgotten variable bindings (e.g., `ask "query" -> unused_result`)
- Missing return statements in flows
- Logic errors where data flows are incomplete

Operations with side effects (memory writes, invocations, communication)
are exempt as they have effects beyond their return value.

This pass is typically run after optimization passes but before artifact
emission, as a diagnostic aid rather than a transformation."#,
    PassCategory::Analysis,
    "mlir::ais::createUnconsumedValueWarningPass()",
);

/// TemplateSpecialization pass - specializes templates with constant inputs.
pub const TEMPLATE_SPECIALIZATION: PassSpec = PassSpec::new(
    "template-specialization",
    "TemplateSpecialization",
    "Specialize templates with constant inputs",
    r#"Identifies LLM operations with constant context inputs and folds them
into the template string at compile time. This reduces runtime token usage
by eliminating redundant context passing.

Example transformation:
  ask("{sys}", [const("System: Be helpful")] {input_names = ["sys"]})
    -> ask("System: Be helpful", [] {input_names = []})

This pass is particularly effective for:
- System prompts and instructions
- Fixed preambles and formatting
- Constant configuration values"#,
    PassCategory::Optimization,
    "mlir::ais::createTemplateSpecializationPass()",
);

/// DeadContextElimination pass - removes unused context inputs.
pub const DEAD_CONTEXT_ELIMINATION: PassSpec = PassSpec::new(
    "dead-context-elimination",
    "DeadContextElimination",
    "Remove unused context inputs from templates",
    r#"Analyzes template strings to identify context placeholders that are never
referenced and removes the corresponding context operands.

Example transformation:
  ask("Question: {q}", [q, junk] {input_names = ["q", "junk"]})
    -> ask("Question: {q}", [q] {input_names = ["q"]})

This reduces token usage and simplifies the graph by eliminating dead data flow."#,
    PassCategory::Optimization,
    "mlir::ais::createDeadContextEliminationPass()",
);

/// PureDeadNodeElimination pass - removes unused provably inert operations.
pub const PURE_DEAD_NODE_ELIMINATION: PassSpec = PassSpec::new(
    "pure-dead-node-elimination",
    "PureDeadNodeElimination",
    "Remove unused provably inert AIS operations",
    r"Removes only a closed allow-list of operations whose result has no uses:
ConstStr, Merge, and WaitAll. The pass also requires MLIR's Pure trait, so it
does not infer purity from operation names or remove authority, replay,
ordering, or observability boundaries.

Identity, Fence, and Nop are never candidates. Repeated removal only collapses
an inert producer chain after every result and control dependency is absent.",
    PassCategory::Optimization,
    "mlir::ais::createPureDeadNodeEliminationPass()",
);

/// SchemaNarrowing pass - reports unproven schema specialization requests.
pub const SCHEMA_NARROWING: PassSpec = PassSpec::new(
    "schema-narrowing",
    "SchemaNarrowing",
    "Explicit-only guarded schema specialization diagnostic",
    r"Retains every output schema unless the compiler has both typed downstream
field-use facts and selected-backend structured-output support evidence.

Generic result-use counts, arbitrary backend names, and unused values do not
prove that a narrower schema preserves the model request contract.

The current IR does not carry both proofs into MLIR. The pass therefore emits
a diagnostic, leaves the schema unchanged, and stays outside automatic
O-level pipelines.",
    PassCategory::Analysis,
    "mlir::ais::createSchemaNarrowingPass()",
);

/// DspyOptimize pass - optimize prompt templates using DSPy.
pub const DSPY_OPTIMIZE: PassSpec = PassSpec::new(
    "dspy-optimize",
    "DspyOptimize",
    "Config-gated compiler prompt optimization",
    r"Invokes DSPy (Stanford NLP) to automatically optimize LLM prompt templates.
Uses MIPROv2, BootstrapFewShot, or COPRO optimizers to discover better
instructions from training examples.

The Rust pipeline injects this pass into O1/O2/O3 only when compiler-owned
prompt tuning configuration and training data are available. Without that
typed request, default O-levels stay deterministic and side-effect free.

Placement: immediately after build-prompt (which synthesizes named placeholders).
Subsequent passes (template-specialization, dead-context-elimination,
prompt-canonicalization) then operate on the optimized templates.",
    PassCategory::Optimization,
    "mlir::ais::createDspyOptimizePass()",
);

/// PromptCanonicalization pass - reorders prompts for shared-prefix reuse.
pub const PROMPT_CANONICALIZATION: PassSpec = PassSpec::new(
    "prompt-canonicalization",
    "PromptCanonicalization",
    "Explicit-only shared-prefix prompt layout experiment",
    r"Analyzes prompt templates across the graph and canonicalizes them to
maximize KV-cache sharing in inference engines that support prefix caching.

This includes:
- Extracting common prompt prefixes
- Reordering context inputs to align prompts
- Normalizing formatting for cache-friendliness

This can help prefix-caching inference backends reuse shared context across
requests. It is explicit-only until prompt layout rewrites are controlled by a
typed backend-agnostic graph-hint contract.",
    PassCategory::Optimization,
    "mlir::ais::createPromptCanonicalizationPass()",
);

// ============================================================================
// Built-in MLIR Passes (not generated, just registered)
// ============================================================================

/// MLIR canonicalizer pass.
pub const CANONICALIZER: PassSpec = PassSpec::new(
    "canonicalizer",
    "Canonicalizer",
    "MLIR canonicalizer (includes DCE)",
    "Standard MLIR canonicalization pass that applies rewrite patterns.",
    PassCategory::Optimization,
    "mlir::createCanonicalizerPass()",
)
.builtin();

/// MLIR CSE pass.
pub const CSE: PassSpec = PassSpec::new(
    "cse",
    "CSE",
    "MLIR common subexpression elimination",
    "Standard MLIR CSE pass. Available through explicit pass lists; not part of default O-levels until LLM purity is typed in the IR.",
    PassCategory::Optimization,
    "mlir::createCSEPass()",
)
.builtin();

/// MLIR symbol DCE pass.
pub const SYMBOL_DCE: PassSpec = PassSpec::new(
    "symbol-dce",
    "SymbolDCE",
    "MLIR symbol dead code elimination",
    "Standard MLIR pass that eliminates unused symbols.",
    PassCategory::Optimization,
    "mlir::createSymbolDCEPass()",
)
.builtin();

// ============================================================================
// All Passes Collection
// ============================================================================

/// All AIS-specific passes (not built-in MLIR passes).
pub const AIS_PASSES: &[&PassSpec] = &[
    &NORMALIZE,
    &BUILD_PROMPT,
    &DSPY_OPTIMIZE,
    &SCHEDULING,
    &ASSIGN_PRIORITY,
    &SHARED_PREFIX_ANALYSIS,
    &FUSE_ASK_OPS,
    &CONDENSE_OPS,
    &UNCONSUMED_VALUE_WARNING,
    &TEMPLATE_SPECIALIZATION,
    &DEAD_CONTEXT_ELIMINATION,
    &PURE_DEAD_NODE_ELIMINATION,
    &SCHEMA_NARROWING,
    &PROMPT_CANONICALIZATION,
];

/// All passes including built-in MLIR passes.
pub const ALL_PASSES: &[&PassSpec] = &[
    // AIS-specific
    &NORMALIZE,
    &BUILD_PROMPT,
    &DSPY_OPTIMIZE,
    &SCHEDULING,
    &ASSIGN_PRIORITY,
    &SHARED_PREFIX_ANALYSIS,
    &FUSE_ASK_OPS,
    &CONDENSE_OPS,
    &UNCONSUMED_VALUE_WARNING,
    &TEMPLATE_SPECIALIZATION,
    &DEAD_CONTEXT_ELIMINATION,
    &PURE_DEAD_NODE_ELIMINATION,
    &SCHEMA_NARROWING,
    &PROMPT_CANONICALIZATION,
    // Built-in MLIR
    &CANONICALIZER,
    &CSE,
    &SYMBOL_DCE,
];

/// Get all pass specifications.
pub fn get_all_passes() -> impl Iterator<Item = &'static PassSpec> {
    ALL_PASSES.iter().copied()
}

/// Get only AIS-specific passes (for TableGen generation).
pub fn get_ais_passes() -> impl Iterator<Item = &'static PassSpec> {
    AIS_PASSES.iter().copied()
}

/// Find a pass by its CLI name.
pub fn find_pass_by_name(name: &str) -> Option<&'static PassSpec> {
    ALL_PASSES.iter().find(|p| p.name == name).copied()
}

// ============================================================================
// Tests
// ============================================================================
