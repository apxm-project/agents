/**
 * @file  DeadContextElimination.cpp
 * @brief Removes user-role context inputs whose names are not referenced in
 *        the template string.
 *
 * Templates carry named placeholders such as `{question}` or `{evidence}`.
 * The parallel `input_names` attribute enumerates the human-readable name
 * of each Data operand in operand order. This pass walks the template,
 * collects every `{name}` placeholder, and drops only user-role operands whose
 * names do not appear. System, dependency-only, tool-context, and control
 * inputs stay in the graph even without textual placeholders. The operand
 * list, `input_names`, and explicit `input_roles` array remain in lock-step.
 *
 * No template renumbering is needed: placeholders reference by name, so
 * removing operands does not change template text.
 *
 * Example transformation:
 *   %r = ais.ask "Use {a} and {c}" [%a, %b, %c : !ais.token]
 *        {input_names = ["a", "b", "c"], input_roles = ["user", "user", "system"]}
 *
 * Becomes:
 *   %r = ais.ask "Use {a} and {c}" [%a, %c : !ais.token]
 *        {input_names = ["a", "c"], input_roles = ["user", "system"]}
 */

#include "ais/Dialect/AIS/Transforms/Passes.h"

#include "ais/Common/Constants.h"
#include "ais/Dialect/AIS/Transforms/Placeholders.h"
#include "PassStatsHelpers.h"

#include "ais/Dialect/AIS/IR/AISOps.h"
#include "ais/Dialect/AIS/Support/AISDebug.h"

#include "mlir/IR/Builders.h"
#include "llvm/ADT/SmallVector.h"
#include "llvm/ADT/StringSet.h"
#include "llvm/ADT/TypeSwitch.h"

namespace mlir::ais {
#define GEN_PASS_DEF_DEADCONTEXTELIMINATION
#include "ais/Dialect/AIS/Transforms/Passes.h.inc"

namespace {

APXM_AIS_DEBUG_SETUP(dead_context)

struct DeadContextEliminationPass : impl::DeadContextEliminationBase<DeadContextEliminationPass> {
  using DeadContextEliminationBase::DeadContextEliminationBase;

  void runOnOperation() override {
    APXM_AIS_DEBUG_HEADER(DeadContextElimination);
    ModuleOp module = getOperation();
    const std::size_t irSizeBefore = computeModuleIRTextLength(module);
    unsigned eliminated = 0;
    unsigned totalContextRemoved = 0;

    module.walk([&](Operation *op) {
      llvm::TypeSwitch<Operation *, void>(op)
          .Case<AskOp>([&](AskOp askOp) {
            auto removed = eliminateDeadContext(askOp);
            if (removed > 0) {
              eliminated++;
              totalContextRemoved += removed;
            }
          })
          .Case<ThinkOp>([&](ThinkOp thinkOp) {
            auto removed = eliminateDeadContext(thinkOp);
            if (removed > 0) {
              eliminated++;
              totalContextRemoved += removed;
            }
          })
          .Case<ReasonOp>([&](ReasonOp reasonOp) {
            auto removed = eliminateDeadContext(reasonOp);
            if (removed > 0) {
              eliminated++;
              totalContextRemoved += removed;
            }
          })
          .Default([](Operation *) {});
    });

    if (eliminated > 0) {
      module->setAttr(apxm::constants::attrs::DEAD_CONTEXT_ELIMINATED,
                      IntegerAttr::get(IntegerType::get(module.getContext(), 64),
                                       totalContextRemoved));
      APXM_AIS_INFO("Eliminated " << totalContextRemoved << " dead context values "
                    "from " << eliminated << " operations");
    } else {
      APXM_AIS_DEBUG("No dead context found");
    }

    // Per-pass stats are drained by apxm_module_drain_pass_stats.
    // fired_count = number of LLM ops that lost at least one context entry.
    const std::size_t irSizeAfter = computeModuleIRTextLength(module);
    const int64_t irDelta = static_cast<int64_t>(irSizeAfter)
                          - static_cast<int64_t>(irSizeBefore);
    writePassStats(module, getArgument(), eliminated, irDelta);

    APXM_AIS_DEBUG_FOOTER(DeadContextElimination);
  }

private:
  /// Eliminate dead context from an LLM operation.
  /// Returns the number of context values deleted.
  template <typename LlmOpT>
  unsigned eliminateDeadContext(LlmOpT op) {
    StringRef templateStr = op.getTemplateStrAttr().getValue();
    const unsigned contextSize = op.getContext().size();

    if (contextSize == 0)
      return 0;

    // Without explicit named placeholders, preserve operands. The context can
    // still be semantically relevant to runtime request construction, and
    // dropping it would also erase graph data-dependency edges.
    if (templateStr.empty() || !templateStr.contains('{')) {
      APXM_AIS_DEBUG("  Template has no placeholders; preserving context");
      return 0;
    }

    // Collect every {name} appearing in the template — dedup by appearance.
    llvm::SmallVector<llvm::StringRef, 8> usedNames =
        placeholders::namesIn(templateStr);
    llvm::StringSet<> usedSet;
    for (llvm::StringRef name : usedNames)
      usedSet.insert(name);

    auto inputNames = placeholders::readInputNames(op.getOperation());

    // Without input_names we cannot make a correct decision: leave op alone.
    if (inputNames.size() != contextSize) {
      APXM_AIS_DEBUG("  input_names mismatch (" << inputNames.size()
                     << " names vs " << contextSize << " operands); skipping");
      return 0;
    }

    const bool hasExplicitInputRoles =
        placeholders::hasInputRoles(op.getOperation());
    auto inputRoles = placeholders::readInputRoles(op.getOperation());
    if (hasExplicitInputRoles &&
        !placeholders::inputRolesAreValid(inputRoles, contextSize)) {
      APXM_AIS_DEBUG("  input_roles mismatch or unsupported value; preserving context");
      return 0;
    }
    if (!hasExplicitInputRoles) {
      inputRoles.reserve(contextSize);
      for (llvm::StringRef inputName : inputNames)
        inputRoles.push_back(placeholders::roleForLegacyInputName(inputName));
    }

    // Walk operands in order, keeping template-referenced user inputs and
    // every protected semantic role.
    SmallVector<Value> newContext;
    llvm::SmallVector<llvm::StringRef, 8> newNames;
    llvm::SmallVector<placeholders::PromptInputRole, 8> newRoles;
    newContext.reserve(contextSize);
    newNames.reserve(contextSize);
    newRoles.reserve(contextSize);
    for (unsigned i = 0; i < contextSize; ++i) {
      const bool keep = !placeholders::isUserRole(inputRoles[i]) ||
                        usedSet.contains(inputNames[i]);
      if (keep) {
        newContext.push_back(op.getContext()[i]);
        newNames.push_back(inputNames[i]);
        newRoles.push_back(inputRoles[i]);
        APXM_AIS_DEBUG("    Keep '" << inputNames[i] << "'");
      } else {
        APXM_AIS_DEBUG("    Remove '" << inputNames[i] << "' (unused)");
      }
    }

    unsigned removed = contextSize - newContext.size();
    if (removed == 0) {
      APXM_AIS_DEBUG("  All context used, no elimination needed");
      return 0;
    }

    OpBuilder builder(op);
    op->setOperands(newContext);
    placeholders::writeInputNames(op.getOperation(), newNames, builder);
    placeholders::writeInputRoles(op.getOperation(), newRoles, builder);

    APXM_AIS_INFO("  Eliminated " << removed << " dead context values "
                  "from template \"" << templateStr << "\"");
    return removed;
  }
};

} // namespace

std::unique_ptr<Pass> createDeadContextEliminationPass() {
  return std::make_unique<DeadContextEliminationPass>();
}

} // namespace mlir::ais
