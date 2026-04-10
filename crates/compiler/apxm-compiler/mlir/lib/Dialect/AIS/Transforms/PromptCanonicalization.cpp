/**
 * @file  PromptCanonicalization.cpp
 * @brief Reorders prompts to maximize shared-prefix reuse for vLLM prefix caching.
 *
 * This pass identifies groups of LLM operations (ask/think/reason) that share
 * common upstream context (same data edges) and reorders their template strings
 * so that shared context appears FIRST in the prompt, followed by unique
 * operation-specific instructions.
 *
 * Example transformation:
 *
 * Before:
 *   %a = ais.ask "As security reviewer, review: {0}" [%diff] : !ais.token
 *   %b = ais.ask "As style reviewer, review: {0}" [%diff] : !ais.token
 *
 * After:
 *   %a = ais.ask "{0}\n---\nReview focus: security" [%diff] : !ais.token
 *        {ais.shared_prefix_group = "diff_review", ais.warmup_candidate = true}
 *   %b = ais.ask "{0}\n---\nReview focus: style" [%diff] : !ais.token
 *        {ais.shared_prefix_group = "diff_review"}
 *
 * This transformation enables vLLM's prefix caching to reuse the expensive
 * shared context (%diff) across multiple operations, reducing repeated prefill
 * work. The pass also emits metadata attributes:
 *
 * - ais.shared_prefix_group: Identifies operations that share a prefix
 * - ais.shared_prefix_est_tokens: Estimated token count of shared prefix
 * - ais.warmup_candidate: Marks the first op in a group for warmup prefill
 *
 * See docs/strategy/09-VLLM-GRAPH-AWARENESS.md section 5.3 for design rationale.
 */

#include "ais/Dialect/AIS/Transforms/Passes.h"

#include "ais/Common/Constants.h"

#include "ais/Dialect/AIS/IR/AISOps.h"
#include "ais/Dialect/AIS/Support/AISDebug.h"

#include "mlir/IR/Builders.h"
#include "llvm/ADT/DenseMap.h"
#include "llvm/ADT/SetVector.h"
#include "llvm/ADT/SmallString.h"
#include "llvm/ADT/StringExtras.h"
#include "llvm/ADT/TypeSwitch.h"

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
  unsigned estimatedTokens;
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

/// Estimate token count for a template string (rough heuristic: 4 chars ≈ 1 token)
static unsigned estimateTokens(StringRef str) {
  return (str.size() + apxm::constants::tokens::CHARS_PER_TOKEN - 1)
         / apxm::constants::tokens::CHARS_PER_TOKEN;
}

/// Extract the instruction-specific part of a template by identifying unique prefix/suffix
static std::pair<StringRef, StringRef> extractInstructionParts(StringRef templateStr) {
  // Find all placeholders to identify template structure
  size_t firstPlaceholder = templateStr.find('{');

  if (firstPlaceholder == StringRef::npos) {
    // No placeholders - entire string is unique instruction
    return {templateStr, ""};
  }

  // Text before first placeholder is unique prefix
  StringRef prefix = templateStr.take_front(firstPlaceholder);

  // Find last placeholder to get suffix
  size_t lastCloseBrace = templateStr.rfind('}');
  StringRef suffix = (lastCloseBrace != StringRef::npos && lastCloseBrace + 1 < templateStr.size())
                       ? templateStr.drop_front(lastCloseBrace + 1)
                       : "";

  return {prefix.trim(), suffix.trim()};
}

struct PromptCanonicalizationPass : impl::PromptCanonicalizationBase<PromptCanonicalizationPass> {
  using PromptCanonicalizationBase::PromptCanonicalizationBase;

  void runOnOperation() override {
    APXM_AIS_DEBUG_HEADER(PromptCanonicalization);
    ModuleOp module = getOperation();

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

      // Estimate shared prefix tokens (sum of all context operands).
      // Use pre-computed BPE counts (ais.est_template_tokens) when available,
      // fall back to chars/4 heuristic for raw .air input.
      unsigned estimatedTokens = 0;
      for (Value v : sharedContext) {
        if (auto* defOp = v.getDefiningOp()) {
          if (auto precomputed = defOp->getAttrOfType<IntegerAttr>(
                  apxm::constants::attrs::EST_TEMPLATE_TOKENS)) {
            estimatedTokens += precomputed.getValue().getZExtValue();
          } else if (auto val = defOp->getAttrOfType<StringAttr>(apxm::constants::attrs::VALUE)) {
            estimatedTokens += estimateTokens(val.getValue());
          } else if (auto tpl = defOp->getAttrOfType<StringAttr>(apxm::constants::attrs::TEMPLATE_STR)) {
            estimatedTokens += estimateTokens(tpl.getValue());
          }
        }
      }
      if (estimatedTokens == 0 && !sharedContext.empty())
        estimatedTokens = 1;

      // Step 3: Reorder each operation's template
      bool isFirst = true;
      for (Operation *op : ops) {
        if (canonicalizePrompt(op, groupName, estimatedTokens, isFirst)) {
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

    APXM_AIS_DEBUG_FOOTER(PromptCanonicalization);
  }

private:
  /// Canonicalize a single LLM operation's prompt template
  /// Returns true if the operation was modified
  bool canonicalizePrompt(Operation *op, StringRef groupName,
                         unsigned estimatedTokens, bool isWarmupCandidate) {
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
    auto [prefix, suffix] = extractInstructionParts(currentTemplate);

    // If there's no unique instruction part, template is already context-first
    if (prefix.empty() && suffix.empty()) {
      APXM_AIS_DEBUG("  Template already context-first: \"" << currentTemplate << "\"");
      return false;
    }

    // Reorder to: {0} (context first) + separator + instruction
    std::string canonicalized;
    llvm::raw_string_ostream os(canonicalized);

    // Emit context placeholders first
    bool first = true;
    size_t contextSize = op->getNumOperands();
    for (size_t i = 0; i < contextSize; ++i) {
      if (!first) os << " ";
      os << "{" << i << "}";
      first = false;
    }

    // Add separator and instruction
    if (!prefix.empty() || !suffix.empty()) {
      os << "\n---\n";
      if (!prefix.empty())
        os << prefix;
      if (!suffix.empty()) {
        if (!prefix.empty())
          os << " ";
        os << suffix;
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
    op->setAttr(apxm::constants::attrs::SHARED_PREFIX_EST_TOKENS,
                builder.getI64IntegerAttr(estimatedTokens));

    if (isWarmupCandidate) {
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
