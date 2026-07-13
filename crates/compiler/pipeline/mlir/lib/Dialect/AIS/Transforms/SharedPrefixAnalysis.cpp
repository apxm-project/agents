/**
 * @file  SharedPrefixAnalysis.cpp
 * @brief Annotates existing shared-prefix opportunities without rewriting prompts.
 *
 * This pass is intentionally analysis-only. It detects LLM operations that
 * already share an identical literal leading prompt segment and emits
 * backend-agnostic reuse metadata. Unlike PromptCanonicalization, it does not
 * move placeholders, mutate templates, or change operands.
 */

#include "ais/Dialect/AIS/Transforms/Passes.h"

#include "ais/Common/Constants.h"
#include "PassStatsHelpers.h"

#include "ais/Dialect/AIS/IR/AISOps.h"
#include "ais/Dialect/AIS/Support/AISDebug.h"
#include "ais/Dialect/AIS/Transforms/Placeholders.h"

#include "mlir/IR/Builders.h"
#include "llvm/ADT/SmallVector.h"
#include "llvm/ADT/StringRef.h"
#include "llvm/ADT/TypeSwitch.h"

#include <algorithm>
#include <optional>
#include <string>

namespace mlir::ais {
#define GEN_PASS_DEF_SHAREDPREFIXANALYSIS
#include "ais/Dialect/AIS/Transforms/Passes.h.inc"

namespace {

APXM_AIS_DEBUG_SETUP(shared_prefix_analysis)

struct PrefixGroup {
  std::string prefixSignature;
  SmallVector<Operation *> ops;
};

static std::optional<StringRef> getTemplate(Operation *op) {
  return TypeSwitch<Operation *, std::optional<StringRef>>(op)
      .Case<AskOp>([](AskOp ask) {
        return ask.getTemplateStrAttr().getValue();
      })
      .Case<ThinkOp>([](ThinkOp think) {
        return think.getTemplateStrAttr().getValue();
      })
      .Case<ReasonOp>([](ReasonOp reason) {
        return reason.getTemplateStrAttr().getValue();
      })
      .Default([](Operation *) -> std::optional<StringRef> {
        return std::nullopt;
      });
}

/// Return the exact static template segment before the first named input.
/// Dynamic operands are intentionally excluded: prefix reuse depends on the
/// identical leading message text, not on estimates attached to their producers.
static std::optional<std::string> leadingStaticPrefixSignature(Operation *op) {
  auto maybeTemplate = getTemplate(op);
  if (!maybeTemplate || maybeTemplate->empty())
    return std::nullopt;

  StringRef templateStr = *maybeTemplate;
  size_t firstDynamicOffset = templateStr.size();
  for (StringRef inputName : placeholders::namesIn(templateStr)) {
    std::string placeholder;
    llvm::raw_string_ostream os(placeholder);
    os << "{" << inputName << "}";
    os.flush();

    const size_t found = templateStr.find(placeholder);
    if (found != StringRef::npos)
      firstDynamicOffset = std::min(firstDynamicOffset, found);
  }

  if (firstDynamicOffset == 0)
    return std::nullopt;

  return templateStr.take_front(firstDynamicOffset).str();
}

struct SharedPrefixAnalysisPass
    : impl::SharedPrefixAnalysisBase<SharedPrefixAnalysisPass> {
  void runOnOperation() override {
    APXM_AIS_DEBUG_HEADER(SharedPrefixAnalysis);
    ModuleOp module = getOperation();
    const std::size_t irSizeBefore = computeModuleIRTextLength(module);

    SmallVector<PrefixGroup> groups;

    module.walk([&](Operation *op) {
      auto maybeTemplate = getTemplate(op);
      if (!maybeTemplate)
        return;

      auto maybePrefix = leadingStaticPrefixSignature(op);
      if (!maybePrefix)
        return;

      for (auto &group : groups) {
        if (group.prefixSignature == *maybePrefix) {
          group.ops.push_back(op);
          return;
        }
      }

      PrefixGroup group;
      group.prefixSignature = std::move(*maybePrefix);
      group.ops.push_back(op);
      groups.push_back(std::move(group));
    });

    unsigned groupsAnnotated = 0;
    unsigned opsAnnotated = 0;
    for (auto &group : groups) {
      if (group.ops.size() < 2)
        continue;

      std::string groupName;
      llvm::raw_string_ostream groupNameOs(groupName);
      groupNameOs << apxm::constants::attrs::SHARED_PREFIX_GROUP_PREFIX
                  << groupsAnnotated;
      groupNameOs.flush();
      bool first = true;
      for (Operation *op : group.ops) {
        OpBuilder builder(op);
        const bool hasPrefixEstimate = op->hasAttr(
            apxm::constants::attrs::SHARED_PREFIX_EST_TOKENS);

        if (!op->hasAttr(apxm::constants::attrs::SHARED_PREFIX_GROUP)) {
          op->setAttr(apxm::constants::attrs::SHARED_PREFIX_GROUP,
                      builder.getStringAttr(groupName));
        }
        if (!op->hasAttr(apxm::constants::attrs::SHARED_PREFIX_GROUP_SIZE)) {
          op->setAttr(apxm::constants::attrs::SHARED_PREFIX_GROUP_SIZE,
                      builder.getI64IntegerAttr(group.ops.size()));
        }
        if (hasPrefixEstimate && first) {
          if (!op->hasAttr(apxm::constants::attrs::WARMUP_CANDIDATE)) {
            op->setAttr(apxm::constants::attrs::WARMUP_CANDIDATE,
                        builder.getBoolAttr(true));
          }
          first = false;
        }

        opsAnnotated++;
      }

      groupsAnnotated++;
    }

    if (opsAnnotated > 0) {
      module->setAttr(apxm::constants::attrs::SHARED_PREFIX_ANALYZED,
                      IntegerAttr::get(IntegerType::get(module.getContext(), 64),
                                       opsAnnotated));
    }

    APXM_AIS_INFO("Annotated " << opsAnnotated << " ops across "
                  << groupsAnnotated << " shared-prefix groups");

    const std::size_t irSizeAfter = computeModuleIRTextLength(module);
    const int64_t irDelta = static_cast<int64_t>(irSizeAfter)
                          - static_cast<int64_t>(irSizeBefore);
    writePassStats(module, getArgument(), opsAnnotated, irDelta);

    APXM_AIS_DEBUG_FOOTER(SharedPrefixAnalysis);
  }
};

}  // namespace

std::unique_ptr<Pass> createSharedPrefixAnalysisPass() {
  return std::make_unique<SharedPrefixAnalysisPass>();
}

}  // namespace mlir::ais
