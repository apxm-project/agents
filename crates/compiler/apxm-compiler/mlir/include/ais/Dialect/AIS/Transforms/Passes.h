/**
 * @file  Passes.h
 * @brief Public entry point for all AIS transformation passes.
 *
 * The file re-exports the auto-generated pass declarations (`GEN_PASS_DECL`)
 * and provides factory functions for each pass.  That keeps the registration
 * site (AisTransforms.cpp) and command-line driver code free of TableGen
 * details.
 *
 * Pass list (in canonical order):
 *   1. normalize                 – canonicalise the graph
 *   2. build-prompt              – generate named placeholders for empty templates
 *   3. scheduling                – annotate with tier/cost/parallel-safe flags
 *   4. fuse-ask-ops              – explicit-only ASK fusion experiment
 *   5. condense-ops              – explicit-only memory batching experiment
 *   6. unconsumed-value-warning  – warn about unused results (DCE)
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

/// Create BuildPrompt pass - generate named placeholders for empty template_str
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

/// Create SchemaNarrowing pass - narrow output schemas based on usage
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
