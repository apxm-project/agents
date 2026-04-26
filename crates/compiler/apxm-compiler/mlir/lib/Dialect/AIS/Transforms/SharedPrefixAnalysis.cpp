/**
 * @file  SharedPrefixAnalysis.cpp
 * @brief Annotates existing shared-prefix opportunities without rewriting prompts.
 *
 * This pass is intentionally analysis-only. It detects LLM operations that
 * already place the same context operands in the same leading prompt segment
 * and emits backend-agnostic reuse metadata. Unlike PromptCanonicalization, it
 * does not move placeholders, mutate templates, or change operands.
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

#include <optional>
#include <string>

namespace mlir::ais {
#define GEN_PASS_DEF_SHAREDPREFIXANALYSIS
#include "ais/Dialect/AIS/Transforms/Passes.h.inc"

namespace {

APXM_AIS_DEBUG_SETUP(shared_prefix_analysis)

struct PrefixCandidate {
  Operation *op = nullptr;
  SmallVector<Value> context;
  std::string prefixSignature;
};

struct PrefixGroup {
  SmallVector<Value> context;
  std::string prefixSignature;
  SmallVector<Operation *> ops;
};

static unsigned estimateTokens(StringRef str) {
  return (str.size() + apxm::constants::tokens::CHARS_PER_TOKEN - 1)
         / apxm::constants::tokens::CHARS_PER_TOKEN;
}

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

static bool sameContext(ArrayRef<Value> lhs, ArrayRef<Value> rhs) {
  if (lhs.size() != rhs.size())
    return false;
  for (size_t idx = 0; idx < lhs.size(); ++idx) {
    if (lhs[idx] != rhs[idx])
      return false;
  }
  return true;
}

static std::optional<std::string> leadingPrefixSignature(Operation *op) {
  auto maybeTemplate = getTemplate(op);
  if (!maybeTemplate || maybeTemplate->empty())
    return std::nullopt;

  auto inputNames = placeholders::readInputNames(op);
  const size_t contextSize = op->getNumOperands();
  if (contextSize == 0 || inputNames.size() != contextSize)
    return std::nullopt;

  StringRef templateStr = *maybeTemplate;
  size_t cursor = 0;
  size_t prefixEnd = 0;
  for (StringRef inputName : inputNames) {
    std::string placeholder;
    llvm::raw_string_ostream os(placeholder);
    os << "{" << inputName << "}";
    os.flush();

    size_t found = templateStr.find(placeholder, cursor);
    if (found == StringRef::npos)
      return std::nullopt;

    prefixEnd = found + placeholder.size();
    cursor = prefixEnd;
  }

  if (prefixEnd == 0)
    return std::nullopt;

  return templateStr.take_front(prefixEnd).trim().str();
}

static unsigned estimateSharedPrefixTokens(Operation *op, StringRef prefixSignature) {
  unsigned estimatedTokens = estimateTokens(prefixSignature);

  for (Value value : op->getOperands()) {
    if (auto *defOp = value.getDefiningOp()) {
      if (auto precomputed = defOp->getAttrOfType<IntegerAttr>(
              apxm::constants::attrs::EST_TEMPLATE_TOKENS)) {
        estimatedTokens += precomputed.getValue().getZExtValue();
      } else if (auto val = defOp->getAttrOfType<StringAttr>(
                     apxm::constants::attrs::VALUE)) {
        estimatedTokens += estimateTokens(val.getValue());
      } else if (auto tpl = defOp->getAttrOfType<StringAttr>(
                     apxm::constants::attrs::TEMPLATE_STR)) {
        estimatedTokens += estimateTokens(tpl.getValue());
      }
    }
  }

  return estimatedTokens == 0 ? 1 : estimatedTokens;
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

      auto maybePrefix = leadingPrefixSignature(op);
      if (!maybePrefix)
        return;

      SmallVector<Value> context(op->getOperands().begin(), op->getOperands().end());

      for (auto &group : groups) {
        if (group.prefixSignature == *maybePrefix &&
            sameContext(group.context, context)) {
          group.ops.push_back(op);
          return;
        }
      }

      PrefixGroup group;
      group.context = std::move(context);
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
        const unsigned estimatedTokens =
            estimateSharedPrefixTokens(op, group.prefixSignature);

        if (!op->hasAttr(apxm::constants::attrs::SHARED_PREFIX_GROUP)) {
          op->setAttr(apxm::constants::attrs::SHARED_PREFIX_GROUP,
                      builder.getStringAttr(groupName));
        }
        if (!op->hasAttr(apxm::constants::attrs::SHARED_PREFIX_EST_TOKENS)) {
          op->setAttr(apxm::constants::attrs::SHARED_PREFIX_EST_TOKENS,
                      builder.getI64IntegerAttr(estimatedTokens));
        }
        if (first && !op->hasAttr(apxm::constants::attrs::WARMUP_CANDIDATE)) {
          op->setAttr(apxm::constants::attrs::WARMUP_CANDIDATE,
                      builder.getBoolAttr(true));
        }

        first = false;
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
