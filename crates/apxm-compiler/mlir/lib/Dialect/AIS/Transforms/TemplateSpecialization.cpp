/**
 * @file  TemplateSpecialization.cpp
 * @brief Specializes template strings with known constant inputs at compile time.
 *
 * When a template string uses placeholders (e.g., "Analyze {0}") and those
 * placeholders reference constant string values, this pass substitutes the
 * constants directly into the template, eliminating runtime interpolation
 * and reducing context overhead.
 *
 * Example transformation:
 *   %const = ais.const_str "user_input_here"
 *   %r = ais.ask "Analyze {0}" [%const : !ais.token]
 *
 * Becomes:
 *   %r = ais.ask "Analyze user_input_here" []
 *
 * The pass only applies when ALL placeholders in the template can be resolved
 * to compile-time constants. If any placeholder references a runtime value,
 * the operation is left unchanged.
 */

#include "ais/Dialect/AIS/Transforms/Passes.h"

#include "ais/Dialect/AIS/IR/AISOps.h"
#include "ais/Dialect/AIS/Support/AISDebug.h"

#include "mlir/IR/Builders.h"
#include "llvm/ADT/SmallString.h"
#include "llvm/ADT/StringExtras.h"
#include "llvm/ADT/TypeSwitch.h"

namespace mlir::ais {
#define GEN_PASS_DEF_TEMPLATESPECIALIZATION
#include "ais/Dialect/AIS/Transforms/Passes.h.inc"

namespace {

APXM_AIS_DEBUG_SETUP(template_specialization)

/// Parse template string to find all placeholder indices (e.g., {0}, {1}, {2})
/// Returns a sorted vector of unique indices found in the template.
static SmallVector<unsigned> extractPlaceholderIndices(StringRef templateStr) {
  SmallVector<unsigned> indices;
  llvm::SmallDenseSet<unsigned> seen;

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
        if (seen.insert(index).second) {
          indices.push_back(index);
        }
      }
      i = closePos; // Skip past the closing brace
    }
  }

  // Sort indices for stable iteration
  llvm::sort(indices);
  return indices;
}

/// Substitute placeholders in template with constant values from context.
/// Returns nullopt if any placeholder cannot be resolved to a constant.
static std::optional<std::string> substituteTemplate(
    StringRef templateStr,
    ValueRange context) {

  // Find all placeholder indices
  auto placeholderIndices = extractPlaceholderIndices(templateStr);

  // Check if all placeholders can be resolved to constants
  SmallVector<std::string> substitutions(context.size());
  for (unsigned idx : placeholderIndices) {
    if (idx >= context.size()) {
      APXM_AIS_DEBUG("  Placeholder {" << idx << "} out of bounds (context size: "
                     << context.size() << ")");
      return std::nullopt;
    }

    // Check if context[idx] is a constant string
    auto defOp = context[idx].getDefiningOp();
    if (!defOp) {
      APXM_AIS_DEBUG("  Context[" << idx << "] has no defining op");
      return std::nullopt;
    }

    if (auto constOp = dyn_cast<ConstStrOp>(defOp)) {
      substitutions[idx] = constOp.getValue().str();
      APXM_AIS_DEBUG("  Context[" << idx << "] = const \"" << substitutions[idx] << "\"");
    } else {
      APXM_AIS_DEBUG("  Context[" << idx << "] is not a constant (op: "
                     << defOp->getName() << ")");
      return std::nullopt;
    }
  }

  // All placeholders are resolvable - perform substitution
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
      unsigned index;
      if (!indexStr.getAsInteger(10, index) && index < substitutions.size()) {
        os << substitutions[index];
        i = closePos; // Skip past the closing brace
        continue;
      }
    }
    os << templateStr[i];
  }

  return os.str();
}

struct TemplateSpecializationPass : impl::TemplateSpecializationBase<TemplateSpecializationPass> {
  using TemplateSpecializationBase::TemplateSpecializationBase;

  void runOnOperation() override {
    APXM_AIS_DEBUG_HEADER(TemplateSpecialization);
    ModuleOp module = getOperation();
    unsigned specialized = 0;

    module.walk([&](Operation *op) {
      llvm::TypeSwitch<Operation *, void>(op)
          .Case<AskOp>([&](AskOp askOp) {
            if (specializeOp(askOp))
              specialized++;
          })
          .Case<ThinkOp>([&](ThinkOp thinkOp) {
            if (specializeOp(thinkOp))
              specialized++;
          })
          .Case<ReasonOp>([&](ReasonOp reasonOp) {
            if (specializeOp(reasonOp))
              specialized++;
          })
          .Default([](Operation *) {});
    });

    if (specialized > 0) {
      module->setAttr("ais.templates_specialized",
                      IntegerAttr::get(IntegerType::get(module.getContext(), 64),
                                       specialized));
      APXM_AIS_INFO("Specialized " << specialized << " template operations");
    } else {
      APXM_AIS_DEBUG("No operations eligible for template specialization");
    }

    APXM_AIS_DEBUG_FOOTER(TemplateSpecialization);
  }

private:
  /// Attempt to specialize an LLM operation's template.
  /// Returns true if the operation was modified.
  template <typename LlmOpT>
  bool specializeOp(LlmOpT op) {
    StringRef templateStr = op.getTemplateStrAttr().getValue();

    // Skip empty templates
    if (templateStr.empty()) {
      APXM_AIS_DEBUG("  Skipping op with empty template");
      return false;
    }

    // Skip templates with no placeholders
    if (!templateStr.contains('{')) {
      APXM_AIS_DEBUG("  Skipping op with no placeholders");
      return false;
    }

    // Skip if no context
    if (op.getContext().empty()) {
      APXM_AIS_DEBUG("  Skipping op with empty context");
      return false;
    }

    APXM_AIS_DEBUG("Attempting to specialize: \"" << templateStr << "\"");

    // Try to substitute all placeholders with constants
    auto specialized = substituteTemplate(templateStr, op.getContext());
    if (!specialized) {
      APXM_AIS_DEBUG("  Cannot specialize (contains runtime values)");
      return false;
    }

    // Update the operation
    OpBuilder builder(op);
    op.setTemplateStrAttr(builder.getStringAttr(*specialized));
    op->setOperands({}); // Clear context since all values are now in template

    APXM_AIS_INFO("  Specialized \"" << templateStr << "\" -> \"" << *specialized << "\"");
    return true;
  }
};

} // namespace

std::unique_ptr<Pass> createTemplateSpecializationPass() {
  return std::make_unique<TemplateSpecializationPass>();
}

} // namespace mlir::ais
