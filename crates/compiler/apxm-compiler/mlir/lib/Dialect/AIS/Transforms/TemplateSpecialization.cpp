/**
 * @file  TemplateSpecialization.cpp
 * @brief Folds compile-time-constant context values into the template
 *        string by name.
 *
 * Templates carry placeholders like `{topic}`. The parallel `input_names`
 * attribute maps each operand to a name. When a placeholder's named
 * operand is produced by an `ais.const_str` op, the constant value is
 * spliced directly into the template — eliminating runtime interpolation
 * and reducing context overhead.
 *
 * Example transformation:
 *   %const = ais.const_str "user_input_here"
 *   %r = ais.ask "Analyze {topic}" [%const : !ais.token]
 *        {input_names = ["topic"]}
 *
 * Becomes:
 *   %r = ais.ask "Analyze user_input_here" [] {input_names = []}
 *
 * The pass only fires when EVERY referenced placeholder resolves to a
 * `ConstStrOp`. If any referenced operand is a runtime value, the op is
 * left unchanged. Operands whose names are not referenced in the template
 * are also left untouched here — DeadContextElimination handles those.
 */

#include "ais/Dialect/AIS/Transforms/Passes.h"

#include "ais/Common/Constants.h"
#include "ais/Dialect/AIS/Transforms/Placeholders.h"

#include "ais/Dialect/AIS/IR/AISOps.h"
#include "ais/Dialect/AIS/Support/AISDebug.h"

#include "mlir/IR/Builders.h"
#include "llvm/ADT/SmallVector.h"
#include "llvm/ADT/StringMap.h"
#include "llvm/ADT/StringSet.h"
#include "llvm/ADT/TypeSwitch.h"

namespace mlir::ais {
#define GEN_PASS_DEF_TEMPLATESPECIALIZATION
#include "ais/Dialect/AIS/Transforms/Passes.h.inc"

namespace {

APXM_AIS_DEBUG_SETUP(template_specialization)

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
      module->setAttr(apxm::constants::attrs::TEMPLATES_SPECIALIZED,
                      IntegerAttr::get(IntegerType::get(module.getContext(), 64),
                                       specialized));
      APXM_AIS_INFO("Specialized " << specialized << " template operations");
    } else {
      APXM_AIS_DEBUG("No operations eligible for template specialization");
    }

    APXM_AIS_DEBUG_FOOTER(TemplateSpecialization);
  }

private:
  /// Attempt to specialize an LLM operation's template by folding constants.
  /// Returns true if the operation was modified.
  template <typename LlmOpT>
  bool specializeOp(LlmOpT op) {
    StringRef templateStr = op.getTemplateStrAttr().getValue();

    if (templateStr.empty() || !templateStr.contains('{')) {
      APXM_AIS_DEBUG("  Skipping op with no placeholders");
      return false;
    }
    if (op.getContext().empty()) {
      APXM_AIS_DEBUG("  Skipping op with empty context");
      return false;
    }

    auto inputNames = placeholders::readInputNames(op.getOperation());
    if (inputNames.size() != op.getContext().size()) {
      APXM_AIS_DEBUG("  Skipping: input_names length mismatch");
      return false;
    }
    auto nameToIdx = placeholders::nameToIndex(inputNames);

    auto referenced = placeholders::namesIn(templateStr);

    // Collect constant substitutions for every referenced name.
    // If ANY name fails to resolve to a ConstStrOp, abort — the op is left
    // untouched.
    llvm::StringMap<std::string> constSubs;
    llvm::StringSet<> referencedSet;
    for (llvm::StringRef name : referenced) {
      referencedSet.insert(name);
      auto it = nameToIdx.find(name);
      if (it == nameToIdx.end()) {
        APXM_AIS_DEBUG("  Placeholder '{" << name
                       << "}' not in input_names; skipping");
        return false;
      }
      Value operand = op.getContext()[it->second];
      Operation *defOp = operand.getDefiningOp();
      auto constOp = defOp ? dyn_cast<ConstStrOp>(defOp) : nullptr;
      if (!constOp) {
        APXM_AIS_DEBUG("  '" << name << "' is not constant; skipping");
        return false;
      }
      constSubs[name] = constOp.getValue().str();
    }

    APXM_AIS_DEBUG("Specializing: \"" << templateStr << "\"");
    std::string newTemplate =
        placeholders::substituteByName(templateStr, constSubs);

    // Drop any operand whose name was specialized; keep the rest untouched.
    SmallVector<Value> remaining;
    llvm::SmallVector<llvm::StringRef, 8> remainingNames;
    for (unsigned i = 0, n = op.getContext().size(); i < n; ++i) {
      if (referencedSet.contains(inputNames[i])) {
        APXM_AIS_DEBUG("    Folded '" << inputNames[i] << "' into template");
        continue;
      }
      remaining.push_back(op.getContext()[i]);
      remainingNames.push_back(inputNames[i]);
    }

    OpBuilder builder(op);
    op.setTemplateStrAttr(builder.getStringAttr(newTemplate));
    op->setOperands(remaining);
    placeholders::writeInputNames(op.getOperation(), remainingNames, builder);

    APXM_AIS_INFO("  Specialized \"" << templateStr << "\" -> \""
                  << newTemplate << "\"");
    return true;
  }
};

} // namespace

std::unique_ptr<Pass> createTemplateSpecializationPass() {
  return std::make_unique<TemplateSpecializationPass>();
}

} // namespace mlir::ais
