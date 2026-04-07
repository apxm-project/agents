/**
 * @file  DeadContextElimination.cpp
 * @brief Removes context inputs that are never referenced in template strings.
 *
 * This pass analyzes template strings to identify which placeholders (e.g., {0}, {1})
 * are actually used, then removes unused context inputs from the operation and
 * renumbers the remaining placeholders to maintain correctness.
 *
 * Example transformation:
 *   %r = ais.ask "Use {0} and {2}" [%a, %b, %c : !ais.token]
 *
 * Becomes:
 *   %r = ais.ask "Use {0} and {1}" [%a, %c : !ais.token]
 *
 * This reduces token costs by eliminating unused context that would otherwise
 * be sent to the LLM. It's particularly effective after fusion passes which may
 * concatenate templates that don't use all inherited context.
 */

#include "ais/Dialect/AIS/Transforms/Passes.h"

#include "ais/Dialect/AIS/IR/AISOps.h"
#include "ais/Dialect/AIS/Support/AISDebug.h"

#include "mlir/IR/Builders.h"
#include "llvm/ADT/DenseSet.h"
#include "llvm/ADT/SmallString.h"
#include "llvm/ADT/StringExtras.h"
#include "llvm/ADT/TypeSwitch.h"

namespace mlir::ais {
#define GEN_PASS_DEF_DEADCONTEXTELIMINATION
#include "ais/Dialect/AIS/Transforms/Passes.h.inc"

namespace {

APXM_AIS_DEBUG_SETUP(dead_context)

/// Parse template string to find all placeholder indices that are actually used.
/// Returns a set of indices found in the template.
static llvm::DenseSet<unsigned> findUsedPlaceholders(StringRef templateStr) {
  llvm::DenseSet<unsigned> used;

  for (size_t i = 0; i < templateStr.size(); ++i) {
    if (templateStr[i] == '{' && i + 1 < templateStr.size()) {
      // Find the closing brace
      size_t closePos = templateStr.find('}', i + 1);
      if (closePos == StringRef::npos)
        continue;

      // Extract the index string
      StringRef indexStr = templateStr.slice(i + 1, closePos);
      unsigned index;
      if (!indexStr.getAsInteger(10, index)) {
        used.insert(index);
      }
      i = closePos; // Skip past the closing brace
    }
  }

  return used;
}

/// Renumber placeholders in template string according to the new index mapping.
/// oldToNew maps old placeholder indices to new indices.
static std::string renumberTemplate(
    StringRef templateStr,
    const llvm::DenseMap<unsigned, unsigned> &oldToNew) {

  std::string result;
  llvm::raw_string_ostream os(result);

  for (size_t i = 0; i < templateStr.size(); ++i) {
    if (templateStr[i] == '{' && i + 1 < templateStr.size()) {
      size_t closePos = templateStr.find('}', i + 1);
      if (closePos == StringRef::npos) {
        os << templateStr[i];
        continue;
      }

      StringRef indexStr = templateStr.slice(i + 1, closePos);
      unsigned oldIndex;
      if (!indexStr.getAsInteger(10, oldIndex)) {
        auto it = oldToNew.find(oldIndex);
        if (it != oldToNew.end()) {
          // Replace with new index
          os << '{' << it->second << '}';
          i = closePos;
          continue;
        }
      }
      // Fall through - output original text if not found
      os << templateStr.substr(i, closePos - i + 1);
      i = closePos;
    } else {
      os << templateStr[i];
    }
  }

  return os.str();
}

struct DeadContextEliminationPass : impl::DeadContextEliminationBase<DeadContextEliminationPass> {
  using DeadContextEliminationBase::DeadContextEliminationBase;

  void runOnOperation() override {
    APXM_AIS_DEBUG_HEADER(DeadContextElimination);
    ModuleOp module = getOperation();
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
      module->setAttr("ais.dead_context_eliminated",
                      IntegerAttr::get(IntegerType::get(module.getContext(), 64),
                                       totalContextRemoved));
      APXM_AIS_INFO("Eliminated " << totalContextRemoved << " dead context values "
                    "from " << eliminated << " operations");
    } else {
      APXM_AIS_DEBUG("No dead context found");
    }

    APXM_AIS_DEBUG_FOOTER(DeadContextElimination);
  }

private:
  /// Eliminate dead context from an LLM operation.
  /// Returns the number of context values removed.
  template <typename LlmOpT>
  unsigned eliminateDeadContext(LlmOpT op) {
    StringRef templateStr = op.getTemplateStrAttr().getValue();

    // Skip empty templates
    if (templateStr.empty()) {
      return 0;
    }

    // Skip templates with no placeholders
    if (!templateStr.contains('{')) {
      // Template has no placeholders but has context - this is dead context
      unsigned contextSize = op.getContext().size();
      if (contextSize > 0) {
        APXM_AIS_DEBUG("Removing " << contextSize << " unused context values "
                       "(template has no placeholders)");
        op->setOperands({});
        return contextSize;
      }
      return 0;
    }

    // Find which placeholders are actually used
    auto usedIndices = findUsedPlaceholders(templateStr);

    // Check if all context is used
    unsigned contextSize = op.getContext().size();
    if (usedIndices.size() == contextSize) {
      // All context is used - check if indices are contiguous [0..n-1]
      bool allUsed = true;
      for (unsigned i = 0; i < contextSize; ++i) {
        if (!usedIndices.contains(i)) {
          allUsed = false;
          break;
        }
      }
      if (allUsed) {
        APXM_AIS_DEBUG("  All context used, no elimination needed");
        return 0;
      }
    }

    APXM_AIS_DEBUG("Template: \"" << templateStr << "\" with "
                   << contextSize << " context values");
    APXM_AIS_DEBUG("  Used indices: " << usedIndices.size() << " placeholders");

    // Build new context with only used values and create index mapping
    SmallVector<Value> newContext;
    llvm::DenseMap<unsigned, unsigned> oldToNew;

    for (unsigned oldIdx = 0; oldIdx < contextSize; ++oldIdx) {
      if (usedIndices.contains(oldIdx)) {
        unsigned newIdx = newContext.size();
        newContext.push_back(op.getContext()[oldIdx]);
        oldToNew[oldIdx] = newIdx;
        APXM_AIS_DEBUG("    Keep context[" << oldIdx << "] -> new index " << newIdx);
      } else {
        APXM_AIS_DEBUG("    Remove context[" << oldIdx << "] (unused)");
      }
    }

    unsigned removed = contextSize - newContext.size();
    if (removed == 0) {
      return 0;
    }

    // Renumber template placeholders
    std::string newTemplate = renumberTemplate(templateStr, oldToNew);

    // Update the operation
    OpBuilder builder(op);
    op.setTemplateStrAttr(builder.getStringAttr(newTemplate));
    op->setOperands(newContext);

    APXM_AIS_INFO("  Eliminated " << removed << " dead context values");
    APXM_AIS_INFO("    Old template: \"" << templateStr << "\"");
    APXM_AIS_INFO("    New template: \"" << newTemplate << "\"");

    return removed;
  }
};

} // namespace

std::unique_ptr<Pass> createDeadContextEliminationPass() {
  return std::make_unique<DeadContextEliminationPass>();
}

} // namespace mlir::ais
