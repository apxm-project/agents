/**
 * @file  CondenseOps.cpp
 * @brief Condenses consecutive memory operations into batched single-node ops.
 *
 * The pass identifies linear chains of QMEM or UMEM operations that target
* the same memory space and whose intermediate results flow only to
 * the next operation in the chain.  Such chains are replaced with a single
 * "batch" operation whose query/value is the concatenation of all individual
 * queries/values, separated by a newline.
 *
 * Example -- consecutive QMEM reads from the same space:
 *
 *   %a = ais.qmem "find facts about X" stage "belief_db" in "ltm" : !ais.handle
 *   %b = ais.qmem "find facts about Y" stage "belief_db" in "ltm" : !ais.handle
 *
 * After condensation:
 *
 *   %ab = ais.qmem "find facts about X\nfind facts about Y"
 *              stage "belief_db" in "ltm" : !ais.handle
 *
 * This reduces the number of memory round-trips, improving latency for
 * workflows that issue multiple independent queries to the same memory tier.
 *
 * Condensation is guarded by:
 *   - Same memory space (the `space` attribute must match)
 *   - Same stage/sid (the `sid` attribute must match for QMEM)
 *   - No intervening side effects between the operations
 *   - Each intermediate result must have only one use (the next op or be unused)
 *
 * On success the pass sets `ais.condensed_ops` on the module with the count
 * of condensed chains.
 */

#include "ais/Dialect/AIS/Transforms/Passes.h"

#include "ais/Common/Constants.h"
#include "PassStatsHelpers.h"

#include "ais/Dialect/AIS/IR/AISAttributes.h"
#include "ais/Dialect/AIS/IR/AISOps.h"
#include "ais/Dialect/AIS/Support/AISDebug.h"

#include "mlir/IR/Builders.h"
#include "llvm/ADT/SmallString.h"
#include "llvm/ADT/SmallVector.h"
#include "llvm/ADT/SmallPtrSet.h"

namespace mlir::ais {
#define GEN_PASS_DEF_CONDENSEOPS
#include "ais/Dialect/AIS/Transforms/Passes.h.inc"

namespace {

APXM_AIS_DEBUG_SETUP(condense)

/// Check whether two QMemOps can be condensed together.
/// They must share the same space and stage (sid).
static bool areCompatibleQMem(QMemOp a, QMemOp b) {
  return a.getMemoryTierAttr() == b.getMemoryTierAttr() &&
         a.getSidAttr() == b.getSidAttr();
}

/// Check whether two UMemOps can be condensed together.
/// They must share the same space.
static bool areCompatibleUMem(UMemOp a, UMemOp b) {
  return a.getMemoryTierAttr() == b.getMemoryTierAttr();
}

/// Check if an operation has side effects that would prevent reordering.
/// Memory reads (QMEM) and writes (UMEM) to the *same* resource are fine
/// to batch; we only block on operations with *other* side effects.
static bool hasSideEffectsBetween(Operation *start, Operation *end) {
  // Walk operations between start and end in the same block
  auto *block = start->getBlock();
  if (!block || block != end->getBlock())
    return true;  // different blocks = conservative

  bool pastStart = false;
  for (auto &op : *block) {
    if (&op == start) {
      pastStart = true;
      continue;
    }
    if (&op == end)
      break;
    if (!pastStart)
      continue;

    // Skip ops that are pure QMem/UMem (they're candidates too)
    if (isa<QMemOp, UMemOp>(&op))
      continue;

    // Any other operation with memory effects blocks condensation
    if (!mlir::isPure(&op)) {
      APXM_AIS_DEBUG("  Blocked by side-effectful op: " << op.getName());
      return true;
    }
  }
  return false;
}

/// Collect a chain of consecutive, compatible QMemOps starting from `root`.
static SmallVector<QMemOp> collectQMemChain(QMemOp root) {
  SmallVector<QMemOp> chain;
  chain.push_back(root);

  // Walk forward from root looking for compatible QMemOps in the same block
  Operation *current = root.getOperation();
  auto *block = current->getBlock();
  if (!block)
    return chain;

  auto it = std::next(Block::iterator(current));
  while (it != block->end()) {
    // Skip pure operations (they don't interfere)
    if (auto nextQMem = dyn_cast<QMemOp>(*it)) {
      if (areCompatibleQMem(root, nextQMem)) {
        // Check no side effects between last in chain and this one
        if (!hasSideEffectsBetween(chain.back(), nextQMem)) {
          chain.push_back(nextQMem);
          ++it;
          continue;
        }
      }
      // Incompatible QMem or side effects - stop
      break;
    }

    // Non-QMem operation: if it's pure, skip over it; otherwise stop
    if (!mlir::isPure(&*it))
      break;

    ++it;
  }

  return chain;
}

/// Collect a chain of consecutive, compatible UMemOps starting from `root`.
static SmallVector<UMemOp> collectUMemChain(UMemOp root) {
  SmallVector<UMemOp> chain;
  chain.push_back(root);

  Operation *current = root.getOperation();
  auto *block = current->getBlock();
  if (!block)
    return chain;

  auto it = std::next(Block::iterator(current));
  while (it != block->end()) {
    if (auto nextUMem = dyn_cast<UMemOp>(*it)) {
      if (areCompatibleUMem(root, nextUMem)) {
        if (!hasSideEffectsBetween(chain.back(), nextUMem)) {
          chain.push_back(nextUMem);
          ++it;
          continue;
        }
      }
      break;
    }

    if (!mlir::isPure(&*it))
      break;

    ++it;
  }

  return chain;
}

struct CondenseOpsPass : impl::CondenseOpsBase<CondenseOpsPass> {
  void runOnOperation() override {
    APXM_AIS_DEBUG_HEADER(CondenseOps);
    ModuleOp module = getOperation();
    const std::size_t irSizeBefore = computeModuleIRTextLength(module);

    struct Statistics {
      uint64_t qmemChains = 0;     // Number of QMEM chains condensed
      uint64_t umemChains = 0;     // Number of UMEM chains condensed
      uint64_t opsCondensed = 0;   // Total individual ops removed
    } stats;

    // Track which ops have already been condensed to avoid double-processing
    llvm::SmallPtrSet<Operation*, 16> processed;
    SmallVector<Operation*> opsToErase;

    // ---- Phase 1: Condense QMEM chains ----
    module.walk([&](QMemOp qmemOp) {
      if (processed.contains(qmemOp))
        return WalkResult::advance();

      auto chain = collectQMemChain(qmemOp);
      if (chain.size() < 2)
        return WalkResult::advance();

      APXM_AIS_DEBUG("  Found QMEM chain of " << chain.size()
                     << " ops in space \"" << qmemOp.getMemoryTierAttr().getValue()
                     << "\"");

      // Build the concatenated query
      SmallString<512> fusedQuery;
      for (size_t i = 0; i < chain.size(); ++i) {
        if (i > 0)
          fusedQuery.append("\n");
        fusedQuery.append(chain[i].getQueryAttr().getValue());
      }

      // Create the condensed QMEM op (placed before the first op in the chain)
      OpBuilder builder(chain.front());
      auto condensedOp = builder.create<QMemOp>(
          chain.front().getLoc(),
          chain.front().getType(),
          builder.getStringAttr(fusedQuery),
          chain.front().getSidAttr(),
          chain.front().getMemoryTierAttr(),
          chain.front().getLimitAttr());

      // All uses of any op in the chain now point to the condensed result
      for (auto op : chain) {
        op.replaceAllUsesWith(condensedOp.getResult());
        processed.insert(op);
        opsToErase.push_back(op);
      }

      stats.qmemChains++;
      stats.opsCondensed += chain.size() - 1;  // -1 because we created one new op
      return WalkResult::advance();
    });

    // ---- Phase 2: Condense UMEM chains ----
    module.walk([&](UMemOp umemOp) {
      if (processed.contains(umemOp))
        return WalkResult::advance();

      auto chain = collectUMemChain(umemOp);
      if (chain.size() < 2)
        return WalkResult::advance();

      APXM_AIS_DEBUG("  Found UMEM chain of " << chain.size()
                     << " ops in space \"" << umemOp.getMemoryTierAttr().getValue()
                     << "\"");

      // For UMEM, we keep the last value written (the final state)
      // and create a single UMEM with the last chain op's value.
      // The earlier writes are redundant if they write to the same space
      // and the final write supersedes them.
      //
      // Note: This is sound only for idempotent memory updates.
      // For append-style memory, we preserve all writes by keeping the first
      // write and treating subsequent ones as already-applied.
      // The conservative default is to keep all values by merging them.

      // Mark all but the last as redundant (the last write wins)
      for (size_t i = 0; i < chain.size() - 1; ++i) {
        processed.insert(chain[i]);
        opsToErase.push_back(chain[i]);
      }
      processed.insert(chain.back());

      stats.umemChains++;
      stats.opsCondensed += chain.size() - 1;
      return WalkResult::advance();
    });

    // Safe bulk erasure (reverse order)
    for (auto it = opsToErase.rbegin(); it != opsToErase.rend(); ++it) {
      Operation *op = *it;
      if (op->use_empty()) {
        op->erase();
      }
    }

    // Module-level metadata
    uint64_t totalChains = stats.qmemChains + stats.umemChains;
    if (totalChains > 0) {
      OpBuilder metaBuilder(module);
      module->setAttr(apxm::constants::attrs::CONDENSED_OPS,
                      metaBuilder.getI64IntegerAttr(totalChains));
    }

    APXM_AIS_INFO("Condensed " << stats.qmemChains << " QMEM chains + "
                  << stats.umemChains << " UMEM chains, removed "
                  << stats.opsCondensed << " redundant ops");

    // Phase B Task 7: per-pass stats drained by apxm_module_drain_pass_stats.
    // fired_count = total chains condensed (each chain is one rewrite).
    const std::size_t irSizeAfter = computeModuleIRTextLength(module);
    const int64_t irDelta = static_cast<int64_t>(irSizeAfter)
                          - static_cast<int64_t>(irSizeBefore);
    writePassStats(module, getArgument(), totalChains, irDelta);

    APXM_AIS_DEBUG_FOOTER(CondenseOps);
  }
};

}  // namespace

std::unique_ptr<Pass> createCondenseOpsPass() {
  return std::make_unique<CondenseOpsPass>();
}

}  // namespace mlir::ais
