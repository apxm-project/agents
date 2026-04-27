/**
 * @file  Passes.td
 * @brief Transform passes that operate on the AIS dialect.
 *
 * Domain-specific transforms:
 *   - normalize      – canonicalize graph structure
 *   - scheduling     – annotate with tier/cost/parallel-safe flags
 *   - fuse-ask-ops   – explicit-only LLM call merge experiment
 *   - unconsumed-value-warning – warn about unused results
 *
 * Each pass declares its constructor, dependent dialects, and tunable options
 * so that `mlir-opt --help` displays them.  Options are wired to the
 * generated pass class via `PassOptions` and can be set on the command line:
 *
 *   mlir-opt --pass-pipeline='builtin.module(ais-fuse-ask-ops)' \
 *            --ais-parallel-threshold=5
 */

#ifndef APXM_AIS_PASSES
#define APXM_AIS_PASSES

include "mlir/Pass/PassBase.td"

//===----------------------------------------------------------------------===//
// Domain-Specific Transform Passes
//===----------------------------------------------------------------------===//

def NormalizeAgentGraph : Pass<"normalize", "mlir::ModuleOp"> {
  let summary = "Normalize AIS graph structure";
  let description = [{
    Canonicalizes the AIS graph by:
    - Deduplicating reasoning contexts
    - Normalizing string attributes (lowercase capability/space names)
    - Establishing SSA ordering invariants for downstream passes

    This pass ensures the IR is in a canonical form that other passes
    can rely on, similar to MLIR's canonicalizer but domain-specific.
  }];
  let constructor = "mlir::ais::createNormalizeAgentGraphPass()";
}

def BuildPrompt : Pass<"build-prompt", "mlir::ModuleOp"> {
  let summary = "Build LLM operation prompt templates";
  let description = [{
    Processes LLM operations and generates placeholder templates:
    1. Empty template_str -> synthesized "{<name>}" templates referencing
       each context operand by its `input_names` entry.
    2. Optionally embed instruction prompts from config.

    Works with InstructionConfig system - does NOT replace runtime mapping,
    just ensures template_str is never empty when context exists.

    This pass enables proper prompt construction for operations like:
      ask(user_input) -> ask("{user_input}", [user_input])
      with the matching `input_names = ["user_input"]` attribute.

    Without this pass, empty template_str causes the runtime to produce
    empty prompts, resulting in empty or broken LLM responses.
  }];
  let constructor = "mlir::ais::createBuildPromptPass()";
  let options = [
    Option<"generatePlaceholders", "generate-placeholders", "bool", "true",
           "Generate named {<input>} placeholders when template_str is empty">,
    Option<"embedInstructions", "embed-instructions", "bool", "false",
           "Embed instruction prompts into artifact (bypasses runtime config)">
  ];
}

def CapabilityScheduling : Pass<"scheduling", "mlir::ModuleOp"> {
  let summary = "Annotate operations with scheduling metadata";
  let description = [{
    Classifies capabilities into execution tiers and annotates operations
    with scheduling hints for the runtime:

    - Tier classification (io/compute/reasoning/memory/general)
    - Cost estimation based on context size
    - Parallel-safety markers for speculation

    These annotations guide the runtime dataflow scheduler to overlap work
    and optimize execution order.
  }];
  let constructor = "mlir::ais::createCapabilitySchedulingPass()";
  let options = [
    Option<"parallelThreshold", "parallel-threshold", "unsigned", /*default=*/"3",
           "Maximum context size considered safe for parallel execution">,
    Option<"baseCost", "base-cost", "unsigned", /*default=*/"2",
           "Base estimated cost for each operation">,
    Option<"contextWeight", "context-weight", "unsigned", /*default=*/"2",
           "Cost multiplier per context operand">
  ];
}

def FuseAskOps : Pass<"fuse-ask-ops", "mlir::ModuleOp"> {
  let summary = "Explicit-only ASK fusion experiment";
  let description = [{
    Identifies producer-consumer ais.ask chains and merges them into single
    batched operations:

    - Reduces serialized LLM API calls
    - Concatenates ask templates with separator
    - Combines contexts from both operations

    Only fuses AskOp (LOW latency). ThinkOp/ReasonOp are not fused because
    they have different semantics (extended thinking, structured output).

    This pass is not part of the automatic O-level pipelines. It must remain
    explicitly requested until APXM has typed request-attribute preservation
    and semantic-quality heuristics for LLM-call merging.

    Only fuses when producer has single use through the consumer chain.
  }];
  let constructor = "mlir::ais::createFuseAskOpsPass()";
  let options = [
    Option<"maxFusionDepth", "max-fusion-depth", "unsigned", /*default=*/"5",
           "Maximum number of operations to fuse into one (0 = unlimited)">,
    Option<"maxTemplateTokens", "max-template-tokens", "unsigned", /*default=*/"2000",
           "Maximum estimated tokens in fused template (0 = unlimited)">,
    Option<"fusionMode", "fusion-mode", "std::string", /*default=*/"\"auto\"",
           "Fusion strategy: 'eager' (always), 'conservative' (short chains), 'auto' (heuristics)">
  ];
}

def CondenseOps : Pass<"condense-ops", "mlir::ModuleOp"> {
  let summary = "Explicit-only memory operation batching experiment";
  let description = [{
    Identifies linear chains of QMEM or UMEM operations that target the same
    memory space and condenses them into a single batched operation.

    For QMEM chains, queries are concatenated (newline-separated) into a single
    query, reducing memory round-trips.  For UMEM chains, redundant intermediate
    writes to the same space are eliminated (last-write-wins).

    Condensation fires when:
    1. Two or more consecutive QMEM/UMEM ops target the same memory space
    2. They share the same stage (sid) for QMEM
    3. No intervening side-effectful operations exist between them
    4. Intermediate results are not consumed by other operations

    This pass is not part of the automatic O-level pipelines until memory-store
    semantics and batching capability contracts are typed.
  }];
  let constructor = "mlir::ais::createCondenseOpsPass()";
}

def AssignPriority : Pass<"assign-priority", "mlir::ModuleOp"> {
  let summary = "Assign execution priority based on critical path analysis";
  let description = [{
    Performs DAG analysis to compute the critical path and assigns priority
    attributes to each AIS operation for the runtime scheduler.

    Priority levels:
    - 90 (Critical): Operations on or near the critical path
    - 70 (High): Operations with high fan-out (3+ consumers)
    - 30 (Normal): All other operations

    The priority attribute is stored as an IntegerAttr and later extracted by
    the ArtifactEmitter into node.metadata.priority, which the runtime scheduler
    uses to assign work to the 4-level priority queue (Critical/High/Normal/Low).

    This pass ensures priority is set even for .air files that bypass Rust
    graph passes, making priority available regardless of the input format.
  }];
  let constructor = "mlir::ais::createAssignPriorityPass()";
}

//===----------------------------------------------------------------------===//
// Analysis/Warning Passes
//===----------------------------------------------------------------------===//

def UnconsumedValueWarning : Pass<"unconsumed-value-warning", "mlir::ModuleOp"> {
  let summary = "Warn about unconsumed operation results";
  let description = [{
    Scans all operations and emits warnings when operation results are not
    consumed by any other operation. This helps developers identify:

    - Forgotten variable bindings (e.g., `rsn "query" -> unused_result`)
    - Missing return statements in flows
    - Logic errors where data flows are incomplete

    Operations with side effects (memory writes, invocations, communication)
    are exempt as they have effects beyond their return value.

    This pass is typically run after optimization passes but before artifact
    emission, as a diagnostic aid rather than a transformation.
  }];
  let constructor = "mlir::ais::createUnconsumedValueWarningPass()";
}

//===----------------------------------------------------------------------===//
// O2+ Optimization Passes
//===----------------------------------------------------------------------===//

def TemplateSpecialization : Pass<"template-specialization", "mlir::ModuleOp"> {
  let summary = "Specialize template strings with known constant inputs";
  let description = [{
    When a template string like "Analyze {input}" is used with a known constant
    input, this pass specializes the template at compile time by substituting
    the constant value directly into the template string.

    Example transformation:
      %const = ais.const_str "user_input_here"
      %r = ais.ask "Analyze {input}" [%const : !ais.token]
           {input_names = ["input"]}

    Becomes:
      %r = ais.ask "Analyze user_input_here" []

    This eliminates runtime template interpolation and reduces context overhead.
    Only applies when ALL template placeholders can be resolved to constants.
  }];
  let constructor = "mlir::ais::createTemplateSpecializationPass()";
}

def DeadContextElimination : Pass<"dead-context-elimination", "mlir::ModuleOp"> {
  let summary = "Remove context inputs that are never referenced in templates";
  let description = [{
    Analyzes template strings to identify which placeholders are actually used,
    then removes unused context inputs from the operation.

    Example transformation:
      %r = ais.ask "Use {a} and {c}" [%a, %b, %c : !ais.token]
           {input_names = ["a", "b", "c"]}

    Becomes:
      %r = ais.ask "Use {a} and {c}" [%a, %c : !ais.token]
           {input_names = ["a", "c"]}

    The operand list and input_names attribute are kept aligned. This pass
    reduces token costs by eliminating unused context that would otherwise be
    sent to the LLM.

    Placeholder-free templates preserve their context operands because those
    operands still carry graph data dependencies and may be consumed by runtime
    request construction.
  }];
  let constructor = "mlir::ais::createDeadContextEliminationPass()";
}

def SchemaNarrowing : Pass<"schema-narrowing", "mlir::ModuleOp"> {
  let summary = "Explicit-only schema narrowing experiment";
  let description = [{
    Planned optimization: when an output_schema attribute is set on a node,
    propagate schema constraints to downstream consumers and narrow overly broad
    schemas based on how the output is actually used.

    Example: If a REASON operation specifies it returns {name, age, address}
    but downstream operations only access 'name' and 'age', the schema can be
    narrowed to {name, age}, reducing token overhead in the LLM response.

    The current implementation is not field-use narrowing, so this pass is not
    part of the automatic O-level pipelines.
  }];
  let constructor = "mlir::ais::createSchemaNarrowingPass()";
}

def DspyOptimize : Pass<"dspy-optimize", "mlir::ModuleOp"> {
  let summary = "Config-gated compiler prompt optimization";
  let description = [{
    Invokes python3 -m apxm_dspy as a subprocess to run DSPy's prompt
    optimizers (MIPROv2, BootstrapFewShot, COPRO) on template strings.

    The Rust pipeline injects this pass into O1/O2/O3 only when compiler-owned
    prompt tuning configuration and training data are available. Without that
    typed request, default O-levels stay deterministic and side-effect free.
  }];
  let constructor = "mlir::ais::createDspyOptimizePass()";
}

def PromptCanonicalization : Pass<"prompt-canonicalization", "mlir::ModuleOp"> {
  let summary = "Explicit-only shared-prefix prompt layout experiment";
  let description = [{
    Identifies groups of LLM operations that share common upstream context and
    reorders their template strings so shared context appears FIRST, followed
    by operation-specific instructions.

    Example transformation:
      Before:
        %a = ais.ask "As security reviewer, review: {diff}" [%diff]
             {input_names = ["diff"]}
        %b = ais.ask "As style reviewer, review: {diff}" [%diff]
             {input_names = ["diff"]}

      After:
        %a = ais.ask "{diff}\n---\nReview focus: security" [%diff]
             {ais.shared_prefix_group = "diff_review", ais.warmup_candidate = true}
        %b = ais.ask "{diff}\n---\nReview focus: style" [%diff]
             {ais.shared_prefix_group = "diff_review"}

    This can help prefix-caching inference backends reuse shared context across
    multiple operations. The pass emits:
    - ais.shared_prefix_group: Identifies operations sharing a prefix
    - ais.shared_prefix_est_tokens: Estimated shared token count
    - ais.warmup_candidate: Marks first op in group for warmup prefill

    This pass is not part of the automatic O-level pipelines until prompt
    layout rewrites are controlled by a typed backend-agnostic graph-hint
    contract.
  }];
  let constructor = "mlir::ais::createPromptCanonicalizationPass()";
}

def SharedPrefixAnalysis : Pass<"shared-prefix-analysis", "mlir::ModuleOp"> {
  let summary = "Annotate existing shared-prefix reuse opportunities";
  let description = [{
    Detects LLM operations that already share the same leading prompt prefix
    and context operands, then emits graph-hint metadata for prefix-aware
    backends.

    This pass does not rewrite prompt templates or reorder operands. It only
    emits analysis metadata:
    - ais.shared_prefix_group: Identifies operations sharing a prefix
    - ais.shared_prefix_est_tokens: Estimated shared token count
    - ais.warmup_candidate: Marks first op in group for warmup prefill
  }];
  let constructor = "mlir::ais::createSharedPrefixAnalysisPass()";
}

#endif // APXM_AIS_PASSES
