/**
 * @file  Passes.td
 * @brief Transform passes that operate on the AIS dialect.
 *
 * Domain-specific transforms:
 *   - normalize      – canonicalize graph structure
 *   - scheduling     – annotate with tier/cost/parallel-safe flags
 *   - fuse-ask-ops   – batch ask LLM calls (highest ROI)
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
    1. Empty template_str -> "{0}" (context[0] becomes prompt)
    2. Optionally embed instruction prompts from config

    Works with InstructionConfig system - does NOT replace runtime mapping,
    just ensures template_str is never empty when context exists.

    This pass enables proper prompt construction for operations like:
      ask(user_input) -> ask("{0}", [user_input])

    Without this pass, empty template_str causes the runtime to produce
    empty prompts, resulting in empty or broken LLM responses.
  }];
  let constructor = "mlir::ais::createBuildPromptPass()";
  let options = [
    Option<"generatePlaceholders", "generate-placeholders", "bool", "true",
           "Generate {0} placeholder when template_str is empty">,
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
  let summary = "Fuse adjacent ask operations to reduce LLM calls";
  let description = [{
    Identifies producer-consumer ais.ask chains and merges them into single
    batched operations. This is the highest-ROI optimization (100-400x) as it:

    - Reduces serialized LLM API calls (each ~500ms-2s)
    - Concatenates ask templates with separator
    - Combines contexts from both operations

    Only fuses AskOp (LOW latency). ThinkOp/ReasonOp are not fused because
    they have different semantics (extended thinking, structured output).

    Fusion is guarded by heuristics to prevent quality degradation:
    - Max fusion depth limits chain length (default: 5)
    - Max template tokens prevents overly long prompts (default: 2000)
    - Operations can opt-out via `ais.no_fuse` attribute

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
  let summary = "Condense consecutive memory operations into batched calls";
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

    This pass is the memory-tier analogue of FuseAskOps for LLM calls.
  }];
  let constructor = "mlir::ais::createCondenseOpsPass()";
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
    When a template string like "Analyze {0}" is used with a known constant
    input, this pass specializes the template at compile time by substituting
    the constant value directly into the template string.

    Example transformation:
      %const = ais.const_str "user_input_here"
      %r = ais.ask "Analyze {0}" [%const : !ais.token]

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
      %r = ais.ask "Use {0} and {2}" [%a, %b, %c : !ais.token]

    Becomes:
      %r = ais.ask "Use {0} and {1}" [%a, %c : !ais.token]

    The placeholder indices are renumbered to maintain correctness. This pass
    reduces token costs by eliminating unused context that would otherwise be
    sent to the LLM.

    This is particularly effective after fusion passes which may concatenate
    templates that don't use all inherited context.
  }];
  let constructor = "mlir::ais::createDeadContextEliminationPass()";
}

def SchemaNarrowing : Pass<"schema-narrowing", "mlir::ModuleOp"> {
  let summary = "Narrow output schemas based on actual usage patterns";
  let description = [{
    When an output_schema attribute is set on a node, this pass propagates
    schema constraints to downstream consumers and narrows overly broad schemas
    based on how the output is actually used.

    Example: If a REASON operation specifies it returns {name, age, address}
    but downstream operations only access 'name' and 'age', the schema can be
    narrowed to {name, age}, reducing token overhead in the LLM response.

    This pass performs data-flow analysis to track which fields are accessed
    and removes unused fields from output_schema attributes.
  }];
  let constructor = "mlir::ais::createSchemaNarrowingPass()";
}

#endif // APXM_AIS_PASSES
