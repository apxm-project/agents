/**
 * @file  BuildPrompt.cpp
 * @brief Materializes positional LLM prompt input contracts for operations
 *        whose context array is non-empty.
 *
 * For example:
 *
 *   DSL:   ask(user_input)
 *   MLIR:  %r = ais.ask "" [%user_input : !ais.token] : !ais.token
 *
 * Without this pass, the empty template_str causes broken prompts at runtime.
 * This pass transforms it to:
 *
 *   %r = ais.ask "{user_input}" [%user_input : !ais.token]
 *        {input_names = ["user_input"], input_roles = ["user"]} : !ais.token
 *
 * Names are taken from the existing `input_names` attribute when present;
 * otherwise the pass falls back to defaults `ctx0`, `ctx1`, ... . Prompt
 * roles are an explicit positional contract: this pass never infers them
 * from names. Operations with missing or malformed role vectors remain
 * unchanged so compiler and artifact boundary validation can reject them.
 *
 * This pass works alongside the InstructionConfig system:
 * - BuildPrompt: Materializes templates and names only after an explicit
 *   `input_roles` contract establishes prompt-channel semantics
 * - InstructionConfig: Maps operation types to system prompts at runtime
 */

#include "ais/Dialect/AIS/Transforms/Passes.h"

#include "ais/Common/Constants.h"
#include "ais/Dialect/AIS/Transforms/Placeholders.h"
#include "PassStatsHelpers.h"

#include "ais/Dialect/AIS/IR/AISOps.h"
#include "ais/Dialect/AIS/Support/AISDebug.h"

#include "mlir/IR/Builders.h"
#include "llvm/ADT/SmallString.h"
#include "llvm/ADT/SmallVector.h"
#include "llvm/ADT/TypeSwitch.h"

namespace mlir::ais {
#define GEN_PASS_DEF_BUILDPROMPT
#include "ais/Dialect/AIS/Transforms/Passes.h.inc"

namespace {

APXM_AIS_DEBUG_SETUP(build_prompt)

struct BuildPromptPass : impl::BuildPromptBase<BuildPromptPass> {
  using BuildPromptBase::BuildPromptBase;

  void runOnOperation() override {
    APXM_AIS_DEBUG_HEADER(BuildPrompt);
    ModuleOp module = getOperation();
    const std::size_t irSizeBefore = computeModuleIRTextLength(module);
    unsigned modified = 0;

    module.walk([&](Operation *op) {
      llvm::TypeSwitch<Operation *>(op)
          .Case<AskOp>([&](AskOp askOp) {
            if (processLlmOp(askOp))
              modified++;
          })
          .Case<ThinkOp>([&](ThinkOp thinkOp) {
            if (processLlmOp(thinkOp))
              modified++;
          })
          .Case<ReasonOp>([&](ReasonOp reasonOp) {
            if (processLlmOp(reasonOp))
              modified++;
          });
    });

    if (modified > 0) {
      module->setAttr(apxm::constants::attrs::PROMPTS_BUILT,
                      IntegerAttr::get(IntegerType::get(module.getContext(), 64),
                                       modified));
      APXM_AIS_INFO("Built prompts for " << modified << " operations");
    } else {
      APXM_AIS_DEBUG("No operations required prompt building");
    }

    // Per-pass stats are drained by apxm_module_drain_pass_stats.
    const std::size_t irSizeAfter = computeModuleIRTextLength(module);
    const int64_t irDelta = static_cast<int64_t>(irSizeAfter)
                          - static_cast<int64_t>(irSizeBefore);
    writePassStats(module, getArgument(), modified, irDelta);

    APXM_AIS_DEBUG_FOOTER(BuildPrompt);
  }

private:
  /// Process an LLM operation (AskOp, ThinkOp, or ReasonOp).
  /// Returns true if the operation was modified.
  template <typename LlmOpT>
  bool processLlmOp(LlmOpT op) {
    StringRef currentTemplate = op.getTemplateStrAttr().getValue();

    if (op.getContext().empty()) {
      APXM_AIS_DEBUG("  Skipping op with empty context");
      return false;
    }

    const unsigned contextSize = op.getContext().size();

    const bool hasExplicitInputRoles =
        placeholders::hasInputRoles(op.getOperation());
    auto inputRoles = placeholders::readInputRoles(op.getOperation());
    if (!hasExplicitInputRoles ||
        !placeholders::inputRolesAreValid(inputRoles, contextSize)) {
      APXM_AIS_DEBUG("  input_roles must be explicit, valid, and positional; "
                     "skipping prompt synthesis");
      return false;
    }

    OpBuilder builder(op);

    // Source the names from the existing input_names attribute if present,
    // otherwise synthesize defaults (ctx0..ctxN-1).
    auto existing = placeholders::readInputNames(op.getOperation());
    llvm::SmallVector<std::string, 8> nameStorage;
    llvm::SmallVector<llvm::StringRef, 8> nameRefs;
    nameStorage.reserve(contextSize);
    nameRefs.reserve(contextSize);
    for (unsigned i = 0; i < contextSize; ++i) {
      if (i < existing.size() && !existing[i].empty()) {
        nameStorage.emplace_back(existing[i].str());
      } else {
        nameStorage.emplace_back(("ctx" + llvm::Twine(i)).str());
      }
      nameRefs.push_back(nameStorage.back());
    }

    bool needsInputNamesWrite = existing.size() != contextSize;
    for (llvm::StringRef name : existing) {
      if (name.empty()) {
        needsInputNamesWrite = true;
        break;
      }
    }

    bool modified = false;
    if (needsInputNamesWrite) {
      placeholders::writeInputNames(op.getOperation(), nameRefs, builder);
      modified = true;
    }

    // Only synthesize template text when the authored template is empty.
    if (currentTemplate.empty()) {
      if (!generatePlaceholders) {
        APXM_AIS_DEBUG("  Placeholder generation disabled");
        return modified;
      }

      // Build placeholders only for user-renderable context. Protected roles
      // remain positional semantic inputs without entering the user channel.
      llvm::SmallString<128> templateBuf;
      bool hasUserContext = false;
      for (unsigned i = 0; i < contextSize; ++i) {
        if (!placeholders::isUserRole(inputRoles[i]))
          continue;
        hasUserContext = true;
        templateBuf.append("{");
        templateBuf.append(nameRefs[i]);
        templateBuf.append("}");
      }
      if (!hasUserContext) {
        APXM_AIS_DEBUG("  No user-role context available for placeholder synthesis");
        return modified;
      }
      op.setTemplateStrAttr(builder.getStringAttr(templateBuf));
      if (!modified)
        placeholders::writeInputNames(op.getOperation(), nameRefs, builder);
      modified = true;

      APXM_AIS_INFO("  Generated named placeholder template for "
                    << op->getName() << " with " << contextSize
                    << " context operands");
    } else if (modified) {
      APXM_AIS_INFO("  Materialized input_names for "
                    << op->getName() << " with " << contextSize
                    << " context operands");
    } else {
      APXM_AIS_DEBUG("  Prompt input contract already materialized");
    }

    return modified;
  }
};

} // namespace

std::unique_ptr<Pass> createBuildPromptPass() {
  return std::make_unique<BuildPromptPass>();
}

} // namespace mlir::ais
