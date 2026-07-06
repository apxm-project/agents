/**
 * @file  PromptCanonicalization.cpp
 * @brief Reorders prompts to maximize shared-prefix reuse for prefix caching.
 *
 * This pass identifies groups of LLM operations (ask/think/reason) that share
 * common upstream context (same data edges) and reorders their template strings
 * so that shared context appears FIRST in the prompt, followed by unique
 * operation-specific instructions.
 *
 * Example transformation:
 *
 * Before:
 *   %a = ais.ask "As security reviewer, review: {diff}" [%diff] : !ais.token
 *   %b = ais.ask "As style reviewer, review: {diff}" [%diff] : !ais.token
 *
 * After:
 *   %a = ais.ask "{diff}\n---\nReview focus: security" [%diff] : !ais.token
 *        {ais.shared_prefix_group = "diff_review", ais.warmup_candidate = true}
 *   %b = ais.ask "{diff}\n---\nReview focus: style" [%diff] : !ais.token
 *        {ais.shared_prefix_group = "diff_review"}
 *
 * This transformation enables prefix-caching inference backends to reuse the expensive
 * shared context (%diff) across multiple operations, reducing repeated prefill
 * work. The pass also emits metadata attributes:
 *
 * - ais.shared_prefix_group: Identifies operations that share a prefix
 * - ais.shared_prefix_est_tokens: Estimated token count of shared prefix
 * - ais.warmup_candidate: Marks the first op in a group for warmup prefill
 *
 * The backend adapter decides whether these backend-agnostic hints map to a
 * concrete serving feature.
 */

#include "ais/Dialect/AIS/Transforms/Passes.h"

#include "ais/Common/Constants.h"
#include "PassStatsHelpers.h"

#include "ais/Dialect/AIS/IR/AISOps.h"
#include "ais/Dialect/AIS/Support/AISDebug.h"
#include "ais/Dialect/AIS/Transforms/Placeholders.h"

#include "mlir/IR/Builders.h"
#include "llvm/ADT/DenseMap.h"
#include "llvm/ADT/SetVector.h"
#include "llvm/ADT/SmallString.h"
#include "llvm/ADT/StringExtras.h"
#include "llvm/ADT/TypeSwitch.h"

#include <optional>

namespace mlir::ais {
#define GEN_PASS_DEF_PROMPTCANONICALIZATION
#include "ais/Dialect/AIS/Transforms/Passes.h.inc"

namespace {

APXM_AIS_DEBUG_SETUP(prompt_canonicalization)

/// Represents a group of operations that share common context
struct ReuseGroup {
  std::string groupName;
  SmallVector<Operation *> ops;
  ValueRange sharedContext;
  std::optional<unsigned> estimatedTokens;
};

/// Hash function for ValueRange to enable DenseMap keying
struct ValueRangeHash {
  size_t operator()(ValueRange range) const {
    size_t hash = 0;
    for (Value v : range) {
      hash ^= llvm::hash_value(v.getAsOpaquePointer()) + 0x9e3779b9 + (hash << 6) + (hash >> 2);
    }
    return hash;
  }
};

/// Equality comparison for ValueRange
struct ValueRangeEqual {
  bool operator()(ValueRange lhs, ValueRange rhs) const {
    if (lhs.size() != rhs.size())
      return false;
    for (size_t i = 0; i < lhs.size(); ++i) {
      if (lhs[i] != rhs[i])
        return false;
    }
    return true;
  }
};

/// Parts extracted from a template around its context placeholders.
struct PromptParts {
  StringRef contextHeader;
  StringRef instructionPrefix;
  StringRef instructionSuffix;
};

/// Extract prompt text around placeholders.
///
/// If the text immediately before the first placeholder is a separate
/// paragraph ending with ':', treat that paragraph as the context header and
/// keep it adjacent to the moved context placeholder. This avoids rewrites like
/// `Shared context: Return ...` after the placeholder is moved to the front.
static PromptParts extractPromptParts(StringRef templateStr) {
  // Find all placeholders to identify template structure
  size_t firstPlaceholder = templateStr.find('{');

  if (firstPlaceholder == StringRef::npos) {
    // No placeholders - entire string is unique instruction
    return {"", templateStr, ""};
  }

  // Text before first placeholder is unique prefix
  StringRef prefix = templateStr.take_front(firstPlaceholder).trim();

  // Find last placeholder to get suffix
  size_t lastCloseBrace = templateStr.rfind('}');
  StringRef suffix = (lastCloseBrace != StringRef::npos && lastCloseBrace + 1 < templateStr.size())
                       ? templateStr.drop_front(lastCloseBrace + 1)
                       : "";

  StringRef contextHeader;
  StringRef instructionPrefix = prefix;

  if (prefix.ends_with(":")) {
    size_t paragraphBreak = prefix.rfind("\n\n");
    if (paragraphBreak != StringRef::npos && paragraphBreak + 2 < prefix.size()) {
      contextHeader = prefix.drop_front(paragraphBreak + 2).trim();
      instructionPrefix = prefix.take_front(paragraphBreak).trim();
    }
  }

  return {contextHeader, instructionPrefix, suffix.trim()};
}

struct PromptCanonicalizationPass : impl::PromptCanonicalizationBase<PromptCanonicalizationPass> {
  using PromptCanonicalizationBase::PromptCanonicalizationBase;

  void runOnOperation() override {
    APXM_AIS_DEBUG_HEADER(PromptCanonicalization);
    ModuleOp module = getOperation();
    const std::size_t irSizeBefore = computeModuleIRTextLength(module);

    // Step 1: Collect all LLM operations and group by shared context
    // Use a map that compares Value pointers for grouping
    llvm::DenseMap<SmallVector<Value>, SmallVector<Operation *>> contextGroups;

    module.walk([&](Operation *op) {
      llvm::TypeSwitch<Operation *, void>(op)
          .Case<AskOp, ThinkOp, ReasonOp>([&](auto llmOp) {
            ValueRange context = llmOp.getContext();
            if (!context.empty()) {
              SmallVector<Value> contextVec(context.begin(), context.end());
              contextGroups[contextVec].push_back(op);
              APXM_AIS_DEBUG("Found LLM op: " << op->getName() << " with "
                             << context.size() << " context operands");
            }
          })
          .Default([](Operation *) {});
    });

    // Step 2: Process groups with 2+ operations (reuse opportunity)
    unsigned groupsProcessed = 0;
    unsigned opsModified = 0;

    for (auto &[sharedContext, ops] : contextGroups) {
      if (ops.size() < 2) {
        APXM_AIS_DEBUG("Skipping group with single op (no reuse opportunity)");
        continue;
      }

      APXM_AIS_INFO("Processing reuse group with " << ops.size() << " operations");

      // Generate group name from context hash
      std::string groupName = "shared_prefix_" + std::to_string(groupsProcessed);

      // Estimate shared prefix tokens only from tokenizer-backed metadata.
      // If a dynamic operand lacks a typed estimate, leave the estimate absent
      // so runtime warmup does not act on a fabricated token count.
      unsigned estimatedTokens = 0;
      bool sawEstimate = false;
      for (Value v : sharedContext) {
        if (auto* defOp = v.getDefiningOp()) {
          if (auto precomputed = defOp->getAttrOfType<IntegerAttr>(
                  apxm::constants::attrs::ESTIMATED_DYNAMIC_TOKENS)) {
            estimatedTokens += precomputed.getValue().getZExtValue();
            sawEstimate = true;
          } else if (auto precomputed = defOp->getAttrOfType<IntegerAttr>(
                         apxm::constants::attrs::EST_TEMPLATE_TOKENS)) {
            estimatedTokens += precomputed.getValue().getZExtValue();
            sawEstimate = true;
          }
        }
      }
      std::optional<unsigned> exactEstimate =
          sawEstimate ? std::optional<unsigned>(estimatedTokens) : std::nullopt;

      // Step 3: Reorder each operation's template
      bool isFirst = true;
      for (Operation *op : ops) {
        if (canonicalizePrompt(op, groupName, exactEstimate, isFirst)) {
          opsModified++;
          isFirst = false; // Only first op is warmup candidate
        }
      }

      groupsProcessed++;
    }

    if (opsModified > 0) {
      module->setAttr(apxm::constants::attrs::PROMPTS_CANONICALIZED,
                      IntegerAttr::get(IntegerType::get(module.getContext(), 64),
                                       opsModified));
      APXM_AIS_INFO("Canonicalized " << opsModified << " prompts in "
                    << groupsProcessed << " reuse groups");
    } else {
      APXM_AIS_DEBUG("No prompts required canonicalization");
    }

    // Per-pass stats are drained by apxm_module_drain_pass_stats.
    const std::size_t irSizeAfter = computeModuleIRTextLength(module);
    const int64_t irDelta = static_cast<int64_t>(irSizeAfter)
                          - static_cast<int64_t>(irSizeBefore);
    writePassStats(module, getArgument(), opsModified, irDelta);

    APXM_AIS_DEBUG_FOOTER(PromptCanonicalization);
  }

private:
  /// Canonicalize a single LLM operation's prompt template
  /// Returns true if the operation was modified
  bool canonicalizePrompt(Operation *op, StringRef groupName,
                         std::optional<unsigned> estimatedTokens,
                         bool isWarmupCandidate) {
    StringRef currentTemplate;

    // Get current template based on operation type
    if (auto askOp = dyn_cast<AskOp>(op)) {
      currentTemplate = askOp.getTemplateStrAttr().getValue();
    } else if (auto thinkOp = dyn_cast<ThinkOp>(op)) {
      currentTemplate = thinkOp.getTemplateStrAttr().getValue();
    } else if (auto reasonOp = dyn_cast<ReasonOp>(op)) {
      currentTemplate = reasonOp.getTemplateStrAttr().getValue();
    } else {
      return false;
    }

    // Skip empty templates
    if (currentTemplate.empty()) {
      APXM_AIS_DEBUG("  Skipping op with empty template");
      return false;
    }

    // Extract instruction-specific parts
    PromptParts parts = extractPromptParts(currentTemplate);

    // If there's no unique instruction part, template is already context-first
    if (parts.contextHeader.empty() && parts.instructionPrefix.empty()
        && parts.instructionSuffix.empty()) {
      APXM_AIS_DEBUG("  Template already context-first: \"" << currentTemplate << "\"");
      return false;
    }

    auto inputNames = placeholders::readInputNames(op);
    size_t contextSize = op->getNumOperands();
    if (inputNames.size() != contextSize) {
      APXM_AIS_DEBUG("  Skipping op with input_names/context mismatch");
      return false;
    }

    // Reorder to: named context placeholders first + separator + instruction
    std::string canonicalized;
    llvm::raw_string_ostream os(canonicalized);

    // Emit context header and placeholders first.
    if (!parts.contextHeader.empty())
      os << parts.contextHeader << "\n";

    bool first = true;
    for (size_t i = 0; i < contextSize; ++i) {
      if (!first) os << " ";
      os << "{" << inputNames[i] << "}";
      first = false;
    }

    // Add separator and instruction
    if (!parts.instructionPrefix.empty() || !parts.instructionSuffix.empty()) {
      os << "\n---\n";
      if (!parts.instructionPrefix.empty())
        os << parts.instructionPrefix;
      if (!parts.instructionSuffix.empty()) {
        if (!parts.instructionPrefix.empty())
          os << "\n\n";
        os << parts.instructionSuffix;
      }
    }

    std::string newTemplate = os.str();

    // Update the operation's template
    OpBuilder builder(op);
    if (auto askOp = dyn_cast<AskOp>(op)) {
      askOp.setTemplateStrAttr(builder.getStringAttr(newTemplate));
    } else if (auto thinkOp = dyn_cast<ThinkOp>(op)) {
      thinkOp.setTemplateStrAttr(builder.getStringAttr(newTemplate));
    } else if (auto reasonOp = dyn_cast<ReasonOp>(op)) {
      reasonOp.setTemplateStrAttr(builder.getStringAttr(newTemplate));
    }

    // Add metadata attributes
    op->setAttr(apxm::constants::attrs::SHARED_PREFIX_GROUP, builder.getStringAttr(groupName));
    if (estimatedTokens) {
      op->setAttr(apxm::constants::attrs::SHARED_PREFIX_EST_TOKENS,
                  builder.getI64IntegerAttr(*estimatedTokens));
    }

    if (estimatedTokens && isWarmupCandidate) {
      op->setAttr(apxm::constants::attrs::WARMUP_CANDIDATE, builder.getBoolAttr(true));
      APXM_AIS_INFO("  Marked as warmup candidate");
    }

    APXM_AIS_INFO("  Canonicalized: \"" << currentTemplate << "\" -> \""
                  << newTemplate << "\" (group: " << groupName << ")");
    return true;
  }
};

} // namespace

std::unique_ptr<Pass> createPromptCanonicalizationPass() {
  return std::make_unique<PromptCanonicalizationPass>();
}

} // namespace mlir::ais
