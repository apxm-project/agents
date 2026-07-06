/**
 * @file  FuseAskOps.cpp
 * @brief Batches producer-consumer `ais.ask` chains into single operations.
 *
 * The pass looks for patterns:
 *   %a = ais.ask "template A" ...
 *   %b = ais.ask "template B" [%a, ...]
 *
 * AND patterns with string interpolation (merge chains):
 *   %a = ais.ask "template A" ...
 *   %s1 = ais.const_str "Using: "
 *   %m1 = ais.merge %s1, %a
 *   %s2 = ais.const_str ", explain..."
 *   %m2 = ais.merge %m1, %s2
 *   %b = ais.ask "" [%m2]
 *
 * and replaces them with one `ais.ask` whose template is the concatenation
 * of all templates and static strings. This pass is explicit-only until APXM
 * has typed request-attribute preservation and semantic-quality heuristics for
 * LLM-call merging.
 *
 * Fusion is guarded by:
 *   - single use chain from producer to consumer (through merges)
 *   - identical dialect attribute compatibility
 *
 * On success the pass increments `ais.fused_pairs` on the module so that
* later stages know how much parallelism changed.
 *
 * Note: Only AskOp is fusible (LOW latency). ThinkOp/ReasonOp are not fused
 * because they have different semantics (extended thinking, structured output).
 */

#include "ais/Dialect/AIS/Transforms/Passes.h"

#include "ais/Common/Constants.h"
#include "ais/Dialect/AIS/Transforms/Placeholders.h"
#include "PassStatsHelpers.h"

#include "ais/Dialect/AIS/IR/AISAttributes.h"
#include "ais/Dialect/AIS/IR/AISOps.h"
#include "ais/Dialect/AIS/Support/AISDebug.h"

#include "mlir/IR/Builders.h"
#include "llvm/ADT/SmallString.h"
#include "llvm/ADT/SmallVector.h"
#include "llvm/ADT/SmallPtrSet.h"
#include "llvm/ADT/StringExtras.h"
#include "llvm/ADT/StringSet.h"

namespace mlir::ais {
#define GEN_PASS_DEF_FUSEASKOPS
#include "ais/Dialect/AIS/Transforms/Passes.h.inc"

namespace {

APXM_AIS_DEBUG_SETUP(fusion)

/// Result of tracing through a merge chain to find an AskOp producer
struct MergeChainTrace {
  AskOp producer;                          // The found AskOp producer
  SmallVector<std::string> stringParts;    // Static strings in order (for template building)
  SmallVector<Operation*> intermediateOps; // MergeOps and ConstStrOps to erase
  SmallVector<Value> otherContext;         // Non-producer context values to preserve
};

/// Trace through merge operations to find an AskOp producer.
/// Returns nullopt if the chain is not fusible (multiple uses, unknown ops, etc.)
static std::optional<MergeChainTrace> traceToAskProducer(Value startOperand, AskOp consumer) {
  MergeChainTrace result;
  SmallVector<std::pair<Value, bool>> worklist; // (value, isBeforeProducer)
  llvm::SmallPtrSet<Operation*, 8> visited;

  // Track string parts with their position indicator
  SmallVector<std::pair<std::string, int>> orderedStrings; // (string, position)
  int posCounter = 0;

  worklist.push_back({startOperand, true});

  while (!worklist.empty()) {
    auto [val, beforeProducer] = worklist.pop_back_val();
    Operation* defOp = val.getDefiningOp();

    if (!defOp)
      continue;

    if (visited.contains(defOp))
      continue;
    visited.insert(defOp);

    if (auto askOp = dyn_cast<AskOp>(defOp)) {
      // Found an AskOp - check if it's fusible
      if (!askOp->hasOneUse()) {
        APXM_AIS_DEBUG("  Not fusible: AskOp has multiple uses");
        return std::nullopt;
      }
      if (result.producer) {
        // Already found a producer - can only fuse one
        APXM_AIS_DEBUG("  Not fusible: Multiple AskOp producers in chain");
        return std::nullopt;
      }
      result.producer = askOp;
      // Collect producer's context
      for (Value ctx : askOp.getOperands()) {
        result.otherContext.push_back(ctx);
      }
    } else if (auto mergeOp = dyn_cast<MergeOp>(defOp)) {
      // MergeOp - check single use and recurse into operands
      if (!mergeOp->hasOneUse()) {
        APXM_AIS_DEBUG("  Not fusible: MergeOp has multiple uses");
        return std::nullopt;
      }
      result.intermediateOps.push_back(mergeOp);
      // Process operands in order (left-to-right for string concatenation)
      for (Value operand : mergeOp.getTokens()) {
        worklist.push_back({operand, beforeProducer});
      }
    } else if (auto constStrOp = dyn_cast<ConstStrOp>(defOp)) {
      // Static string - collect for template building
      orderedStrings.push_back({constStrOp.getValue().str(), posCounter++});
      result.intermediateOps.push_back(constStrOp);
    } else {
      // Unknown operation in chain - not fusible
      APXM_AIS_DEBUG("  Not fusible: Unknown op in chain: " << defOp->getName());
      return std::nullopt;
    }
  }

  if (!result.producer) {
    return std::nullopt;
  }

  // Sort strings by position and extract
  llvm::sort(orderedStrings, [](const auto& a, const auto& b) {
    return a.second < b.second;
  });
  for (const auto& [str, _] : orderedStrings) {
    result.stringParts.push_back(str);
  }

  return result;
}

// Declarative fusion condition predicate (direct connection)
static bool isFusibleProducer(AskOp producer, AskOp consumer) {
  return producer && producer->hasOneUse() &&
         producer->getUses().begin()->getOwner() == consumer.getOperation();
}

// Template fusion strategy - combines producer template, interpolation strings, and consumer template
static std::string fuseTemplates(StringRef producerTemplate,
                                  ArrayRef<std::string> interpolationStrings,
                                  StringRef consumerTemplate) {
  SmallString<256> fused;
  fused.append(producerTemplate);
  fused.append("\n---\n");

  // Add interpolation strings (these were the string concat parts)
  for (const auto& str : interpolationStrings) {
    fused.append(str);
  }

  if (!consumerTemplate.empty()) {
    fused.append(consumerTemplate);
  }

  return std::string(fused);
}

// Direct template fusion for connections without merge chains.
static std::string fuseTemplates(StringRef producerTemplate, StringRef consumerTemplate) {
  return fuseTemplates(producerTemplate, {}, consumerTemplate);
}

/// If the consumer template references the producer by name (i.e. there is a
/// `{<consumerNameForProducer>}` placeholder), substitute the producer's
/// template content inline at that placeholder and return the rewritten
/// template. The producer is then no longer prepended with `\n---\n`, since
/// its contribution is already woven into the consumer's text where the
/// consumer originally expected the producer's result.
///
/// Returns std::nullopt when no substitution is needed: either the consumer
/// did not name the producer, or the placeholder did not appear in the text.
/// Callers fall back to the concat-based `fuseTemplates` in that case.
static std::optional<std::string> substituteProducerInConsumer(
    StringRef producerTemplate,
    StringRef consumerTemplate,
    StringRef consumerNameForProducer) {
  if (consumerNameForProducer.empty())
    return std::nullopt;
  llvm::StringMap<std::string> replacements;
  replacements[consumerNameForProducer] = producerTemplate.str();
  std::string substituted =
      placeholders::substituteByName(consumerTemplate, replacements);
  if (substituted == consumerTemplate.str())
    return std::nullopt;
  return substituted;
}

/// Locate the consumer input slot whose operand equals `consumed`. Returns
/// the consumer's parallel `input_names` entry for that slot, or an empty
/// StringRef when there is no match (which makes downstream substitution a
/// no-op).
static StringRef findConsumerNameForOperand(
    ValueRange consumerOperands,
    llvm::ArrayRef<llvm::StringRef> consumerNames,
    Value consumed) {
  for (size_t i = 0;
       i < consumerOperands.size() && i < consumerNames.size(); ++i) {
    if (consumerOperands[i] == consumed)
      return consumerNames[i];
  }
  return StringRef();
}

/// Merge two input_names arrays consistently with how operands are merged.
/// `producerNames` are the producer's input slots (kept entirely).
/// `consumerNames` are the consumer's input slots, paired with
/// `consumerOperands` so we can drop the entry that corresponded to the
/// producer's now-inlined result.
static llvm::SmallVector<std::string, 8>
mergeInputNames(llvm::ArrayRef<llvm::StringRef> producerNames,
                llvm::ArrayRef<llvm::StringRef> consumerNames,
                ValueRange consumerOperands,
                Value consumedValue) {
  llvm::SmallVector<std::string, 8> merged;
  merged.reserve(producerNames.size() + consumerNames.size());
  llvm::StringSet<> taken;
  auto pushUnique = [&](llvm::StringRef base) {
    std::string candidate = base.str();
    unsigned suffix = 1;
    while (!taken.insert(candidate).second) {
      candidate = (base + llvm::Twine("_") + llvm::Twine(suffix++)).str();
    }
    merged.emplace_back(std::move(candidate));
  };
  for (llvm::StringRef name : producerNames)
    pushUnique(name);
  for (size_t i = 0; i < consumerNames.size(); ++i) {
    if (i < consumerOperands.size() && consumerOperands[i] == consumedValue)
      continue;
    pushUnique(consumerNames[i]);
  }
  return merged;
}

struct FuseAskOpsPass : impl::FuseAskOpsBase<FuseAskOpsPass> {
  void runOnOperation() override {
    APXM_AIS_DEBUG_HEADER(FuseAskOps);
    ModuleOp module = getOperation();
    const std::size_t irSizeBefore = computeModuleIRTextLength(module);

    struct Statistics {
      uint64_t scanned = 0;
      uint64_t fusedDirect = 0;      // Direct ask->ask fusion
      uint64_t fusedMergeChain = 0;  // Fusion through merge chains
      uint64_t skippedBudget = 0;    // Skipped due to token budget
    } stats;

    SmallVector<Operation*> opsToErase;

    // Read pre-computed BPE token count from op. If the compiler has no
    // tokenizer-backed estimate, budget-gated fusion must skip the rewrite
    // rather than inventing a heuristic count.
    auto estimateOpTokens = [](Operation *op) -> std::optional<size_t> {
      if (auto precomputed = op->getAttrOfType<IntegerAttr>(
              apxm::constants::attrs::EST_TEMPLATE_TOKENS))
        return precomputed.getValue().getZExtValue();
      return std::nullopt;
    };

    // Single-pass fusion with clear termination conditions
    module.walk([&](AskOp consumer) {
      stats.scanned++;

      // First, try direct fusion when an ask result is used directly by a consumer.
      auto directProducer = llvm::find_if(consumer.getOperands(), [&](Value operand) {
        return isFusibleProducer(operand.getDefiningOp<AskOp>(), consumer);
      });

      if (directProducer != consumer.getOperands().end()) {
        // Direct fusion path
        AskOp producer = (*directProducer).getDefiningOp<AskOp>();

        if (maxTemplateTokens > 0) {
          auto producerTokens = estimateOpTokens(producer);
          auto consumerTokens = estimateOpTokens(consumer);
          if (!producerTokens || !consumerTokens) {
            APXM_AIS_DEBUG("  Skipping fusion: missing tokenizer-backed token estimate");
            stats.skippedBudget++;
            return WalkResult::advance();
          }
          size_t fusedTokens = *producerTokens + *consumerTokens;
          if (fusedTokens > maxTemplateTokens) {
            APXM_AIS_DEBUG("  Skipping fusion: estimated " << fusedTokens
                          << " tokens > limit " << maxTemplateTokens);
            stats.skippedBudget++;
            return WalkResult::advance();
          }
        }

        APXM_AIS_DEBUG("  Direct fusion: [" << producer.getTemplateStrAttr() << "] + ["
                                            << consumer.getTemplateStrAttr() << "]");

        SmallVector<Value> fusedContext;
        fusedContext.reserve(producer.getOperands().size() + consumer.getOperands().size() - 1);
        llvm::append_range(fusedContext, producer.getOperands());
        llvm::copy_if(consumer.getOperands(), std::back_inserter(fusedContext),
                      [&](Value ctx) { return ctx != producer.getResult(); });

        OpBuilder builder(consumer);

        auto producerNames = placeholders::readInputNames(producer.getOperation());
        auto consumerNames = placeholders::readInputNames(consumer.getOperation());

        StringRef consumerNameForProducer = findConsumerNameForOperand(
            consumer.getOperands(), consumerNames, producer.getResult());
        auto substituted = substituteProducerInConsumer(
            producer.getTemplateStrAttr().getValue(),
            consumer.getTemplateStrAttr().getValue(),
            consumerNameForProducer);
        std::string fusedTemplate = substituted
            ? std::move(*substituted)
            : fuseTemplates(producer.getTemplateStrAttr().getValue(),
                            consumer.getTemplateStrAttr().getValue());

        auto mergedNames = mergeInputNames(producerNames, consumerNames,
                                           consumer.getOperands(),
                                           producer.getResult());

        auto fusedOp = builder.create<AskOp>(
            consumer.getLoc(), consumer.getType(),
            builder.getStringAttr(fusedTemplate), fusedContext);

        // Transfer attributes (input_names is rebuilt below from the merged
        // operand list, so skip the consumer's stale copy here).
        for (NamedAttribute attr : consumer->getAttrs()) {
          if (attr.getName() != apxm::constants::attrs::TEMPLATE_STR &&
              attr.getName() != apxm::constants::attrs::INPUT_NAMES &&
              attr.getName() != "operandSegmentSizes") {
            fusedOp->setAttr(attr.getName(), attr.getValue());
          }
        }

        llvm::SmallVector<llvm::StringRef, 8> mergedRefs;
        mergedRefs.reserve(mergedNames.size());
        for (const auto &name : mergedNames)
          mergedRefs.push_back(name);
        placeholders::writeInputNames(fusedOp.getOperation(), mergedRefs, builder);

        fusedOp->setAttr(apxm::constants::attrs::FUSED_FROM,
          AISFusedFromAttr::get(module.getContext(),
            builder.getArrayAttr({
              builder.getStringAttr(llvm::join_items(".", "producer", producer.getTemplateStrAttr())),
              builder.getStringAttr(llvm::join_items(".", "consumer", consumer.getTemplateStrAttr()))
            })));

        consumer.replaceAllUsesWith(fusedOp.getResult());
        opsToErase.push_back(consumer);
        opsToErase.push_back(producer);
        stats.fusedDirect++;
        return WalkResult::advance();
      }

      // Second, try fusion through merge chains (string interpolation patterns)
      for (Value operand : consumer.getOperands()) {
        // Skip if operand is directly an AskOp (would have been caught above)
        if (operand.getDefiningOp<AskOp>())
          continue;

        // Check if operand comes from a merge chain containing an AskOp
        if (auto trace = traceToAskProducer(operand, consumer)) {
          AskOp producer = trace->producer;

          if (maxTemplateTokens > 0) {
            auto producerTokens = estimateOpTokens(producer);
            auto consumerTokens = estimateOpTokens(consumer);
            if (!producerTokens || !consumerTokens || !trace->stringParts.empty()) {
              APXM_AIS_DEBUG("  Skipping merge chain fusion: missing tokenizer-backed token estimate");
              stats.skippedBudget++;
              return WalkResult::advance();
            }
            size_t fusedTokens = *producerTokens + *consumerTokens;

            if (fusedTokens > maxTemplateTokens) {
              APXM_AIS_DEBUG("  Skipping merge chain fusion: estimated " << fusedTokens
                            << " tokens > limit " << maxTemplateTokens);
              stats.skippedBudget++;
              return WalkResult::advance();
            }
          }

          APXM_AIS_DEBUG("  Merge chain fusion: [" << producer.getTemplateStrAttr()
                         << "] + " << trace->stringParts.size() << " strings + ["
                         << consumer.getTemplateStrAttr() << "]");

          // Build fused context: producer's context + consumer's other context
          SmallVector<Value> fusedContext;
          fusedContext.reserve(trace->otherContext.size() + consumer.getOperands().size());
          llvm::append_range(fusedContext, trace->otherContext);
          llvm::copy_if(consumer.getOperands(), std::back_inserter(fusedContext),
                        [&](Value ctx) { return ctx != operand; });

          OpBuilder builder(consumer);

          auto producerNames = placeholders::readInputNames(producer.getOperation());
          auto consumerNames = placeholders::readInputNames(consumer.getOperation());

          // If the consumer template references the merge result by name,
          // substitute its placeholder with the merge chain's expanded text
          // (producer template + interpolation strings). Otherwise fall back
          // to the prepend-with-separator form.
          std::string mergeChainExpansion;
          {
            mergeChainExpansion.append(
                producer.getTemplateStrAttr().getValue().str());
            for (const auto &str : trace->stringParts)
              mergeChainExpansion.append(str);
          }
          StringRef consumerNameForOperand = findConsumerNameForOperand(
              consumer.getOperands(), consumerNames, operand);
          auto substituted = substituteProducerInConsumer(
              mergeChainExpansion,
              consumer.getTemplateStrAttr().getValue(),
              consumerNameForOperand);
          std::string fusedTemplate = substituted
              ? std::move(*substituted)
              : fuseTemplates(producer.getTemplateStrAttr().getValue(),
                              trace->stringParts,
                              consumer.getTemplateStrAttr().getValue());

          // The consumed value here is the final operand of the merge chain
          // (i.e. the one that flows into the consumer through `operand`).
          auto mergedNames = mergeInputNames(producerNames, consumerNames,
                                             consumer.getOperands(), operand);

          auto fusedOp = builder.create<AskOp>(
              consumer.getLoc(), consumer.getType(),
              builder.getStringAttr(fusedTemplate), fusedContext);

          // Transfer attributes (input_names is rebuilt below from the merged
          // operand list, so skip the consumer's stale copy here).
          for (NamedAttribute attr : consumer->getAttrs()) {
            if (attr.getName() != apxm::constants::attrs::TEMPLATE_STR &&
                attr.getName() != apxm::constants::attrs::INPUT_NAMES &&
                attr.getName() != "operandSegmentSizes") {
              fusedOp->setAttr(attr.getName(), attr.getValue());
            }
          }

          llvm::SmallVector<llvm::StringRef, 8> mergedRefs;
          mergedRefs.reserve(mergedNames.size());
          for (const auto &name : mergedNames)
            mergedRefs.push_back(name);
          placeholders::writeInputNames(fusedOp.getOperation(), mergedRefs, builder);

          fusedOp->setAttr(apxm::constants::attrs::FUSED_FROM,
            AISFusedFromAttr::get(module.getContext(),
              builder.getArrayAttr({
                builder.getStringAttr(llvm::join_items(".", "producer", producer.getTemplateStrAttr())),
                builder.getStringAttr("merge_chain"),
                builder.getStringAttr(llvm::join_items(".", "consumer", consumer.getTemplateStrAttr()))
              })));

          consumer.replaceAllUsesWith(fusedOp.getResult());
          opsToErase.push_back(consumer);
          opsToErase.push_back(producer);
          // Also mark intermediate ops for erasure
          for (Operation* op : trace->intermediateOps) {
            opsToErase.push_back(op);
          }
          stats.fusedMergeChain++;
          return WalkResult::advance();
        }
      }

      return WalkResult::advance();
    });

    // Safe bulk erasure (reverse order to handle dependencies)
    for (auto it = opsToErase.rbegin(); it != opsToErase.rend(); ++it) {
      Operation* op = *it;
      if (op->use_empty()) {
        op->erase();
      }
    }

    // Module-level metadata
    uint64_t totalFused = stats.fusedDirect + stats.fusedMergeChain;
    module->setAttr(apxm::constants::attrs::FUSED_PAIRS,
                    AISFusedPairsAttr::get(module.getContext(), totalFused));

    // Per-pass stats are drained by apxm_module_drain_pass_stats.
    const std::size_t irSizeAfter = computeModuleIRTextLength(module);
    const int64_t irDelta = static_cast<int64_t>(irSizeAfter)
                          - static_cast<int64_t>(irSizeBefore);
    writePassStats(module, getArgument(), totalFused, irDelta);

    APXM_AIS_INFO("Scanned " << stats.scanned << " ASK ops, fused "
                  << stats.fusedDirect << " direct + "
                  << stats.fusedMergeChain << " merge chains = "
                  << totalFused << " total"
                  << (stats.skippedBudget > 0
                      ? " (skipped " + std::to_string(stats.skippedBudget) + " due to token budget)"
                      : ""));
    APXM_AIS_DEBUG_FOOTER(FuseAskOps);
  }
};

}  // namespace

std::unique_ptr<Pass> createFuseAskOpsPass() {
  return std::make_unique<FuseAskOpsPass>();
}

}  // namespace mlir::ais
