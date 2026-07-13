/**
 * @file  Passes.h
 * @brief Public entry point for all AIS transformation passes.
 *
 * The file re-exports the auto-generated pass declarations (`GEN_PASS_DECL`)
 * and provides factory functions for each pass.  That keeps the registration
 * site (AisTransforms.cpp) and command-line driver code free of TableGen
 * details.
 *
 * Pass list (canonical registration order):
 *   1. normalize                  – canonicalize the graph
 *   2. build-prompt               – materialize LLM template/input_names contracts
 *   3. template-specialization    – fold known constants into templates
 *   4. dead-context-elimination   – remove context operands unused by templates
 *   5. pure-dead-node-elimination – erase unused provably inert nodes
 *   6. scheduling                 – annotate with tier/cost/parallel-safe flags
 *   7. shared-prefix-analysis     – annotate existing prefix-reuse opportunities
 *   8. assign-priority            – stamp critical-path priority metadata
 *   9. dspy-optimize              – config-gated prompt-template tuning
 *  10. unconsumed-value-warning   – warn about unused results
 *  11. explicit-only experiments  – fuse, condense, schema, prompt canonicalization
 */

#ifndef APXM_AIS_PASSES_H
#define APXM_AIS_PASSES_H

#include "mlir/IR/BuiltinOps.h"
#include "mlir/Pass/Pass.h"
#include <memory>

namespace mlir::ais {

//===----------------------------------------------------------------------===//
// Pass Declarations (generated from Passes.td)
//===----------------------------------------------------------------------===//

#define GEN_PASS_DECL
#include "ais/Dialect/AIS/Transforms/Passes.h.inc"

//===----------------------------------------------------------------------===//
// Pass Creation Functions
//===----------------------------------------------------------------------===//

/// Create NormalizeAgentGraph pass - canonicalize AIS graph structure
std::unique_ptr<Pass> createNormalizeAgentGraphPass();

/// Create BuildPrompt pass - materialize LLM template/input_names contracts
std::unique_ptr<Pass> createBuildPromptPass();

/// Create CapabilityScheduling pass - annotate with scheduling metadata
std::unique_ptr<Pass> createCapabilitySchedulingPass();

/// Create FuseAskOps pass - explicit-only ASK fusion experiment
std::unique_ptr<Pass> createFuseAskOpsPass();

/// Create CondenseOps pass - batch consecutive memory operations
std::unique_ptr<Pass> createCondenseOpsPass();

/// Create UnconsumedValueWarning pass - warn about unused operation results
std::unique_ptr<Pass> createUnconsumedValueWarningPass();

/// Create TemplateSpecialization pass - specialize templates with constant inputs
std::unique_ptr<Pass> createTemplateSpecializationPass();

/// Create DeadContextElimination pass - remove unused context inputs
std::unique_ptr<Pass> createDeadContextEliminationPass();

/// Create PureDeadNodeElimination pass - remove unused inert operations
std::unique_ptr<Pass> createPureDeadNodeEliminationPass();

/// Create SchemaNarrowing pass - diagnose unproven schema specialization
std::unique_ptr<Pass> createSchemaNarrowingPass();

/// Create PromptCanonicalization pass - reorder prompts for shared-prefix reuse
std::unique_ptr<Pass> createPromptCanonicalizationPass();

/// Create SharedPrefixAnalysis pass - annotate existing shared-prefix reuse
std::unique_ptr<Pass> createSharedPrefixAnalysisPass();

/// Create AssignPriority pass - assign execution priority based on critical path
std::unique_ptr<Pass> createAssignPriorityPass();

/// Create DspyOptimize pass - optimize templates using DSPy
std::unique_ptr<Pass> createDspyOptimizePass();

//===----------------------------------------------------------------------===//
// Pass Registration
//===----------------------------------------------------------------------===//

/// Register all AIS passes with the global registry.
/// This enables passes to be used via textual pass pipeline syntax.
void registerAISPasses();

}  // namespace mlir::ais

#endif  // APXM_AIS_PASSES_H
