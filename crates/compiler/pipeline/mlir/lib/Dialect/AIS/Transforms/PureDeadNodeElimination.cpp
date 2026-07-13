/**
 * @file  PureDeadNodeElimination.cpp
 * @brief Removes unused AIS operations with a closed inertness proof.
 *
 * This pass owns operation-level dead-node removal for the AIS dialect. It
 * intentionally uses a closed allow-list rather than guessing from operation
 * names or result types: only `ais.const_str`, `ais.merge`, and
 * `ais.wait_all` may be removed, and each still has to retain MLIR's `Pure`
 * trait and an unused result.
 *
 * Before:
 * ```mlir
 * %left = ais.const_str "unused" : !ais.token
 * %joined = ais.wait_all %left : !ais.token -> !ais.token
 * ```
 *
 * After:
 * ```mlir
 * // Both operations are removed because `%joined` has no use.
 * ```
 */

#include "ais/Dialect/AIS/Transforms/Passes.h"

#include "PassStatsHelpers.h"

#include "ais/Dialect/AIS/IR/AISOps.h"

#include "mlir/Interfaces/SideEffectInterfaces.h"
#include "mlir/IR/Operation.h"
#include "llvm/ADT/SmallVector.h"

namespace mlir::ais {
#define GEN_PASS_DEF_PUREDEADNODEELIMINATION
#include "ais/Dialect/AIS/Transforms/Passes.h.inc"

namespace {

/// Return whether an operation belongs to the closed inert allow-list.
bool isAllowedPureNode(Operation *op) {
  if (!isa<ConstStrOp, MergeOp, WaitAllOp>(op))
    return false;

  // The explicit trait check keeps the pass fail-closed if an allow-listed
  // operation ever acquires a typed side effect.
  return isPure(op);
}

/// Return whether `op` has the only shape this pass can erase safely.
bool isUnusedAllowedPureNode(Operation *op) {
  return isAllowedPureNode(op) && op->getNumResults() == 1 &&
         op->getResult(0).use_empty();
}

struct PureDeadNodeEliminationPass
    : impl::PureDeadNodeEliminationBase<PureDeadNodeEliminationPass> {
  using PureDeadNodeEliminationBase::PureDeadNodeEliminationBase;

  void runOnOperation() override {
    ModuleOp module = getOperation();
    const std::size_t irSizeBefore = computeModuleIRTextLength(module);
    unsigned eliminated = 0;

    // Each erasure can make an upstream allowed producer unused. Continue
    // until no candidate remains; every iteration strictly removes operations.
    while (true) {
      llvm::SmallVector<Operation *> deadNodes;
      module.walk([&](Operation *op) {
        if (isUnusedAllowedPureNode(op))
          deadNodes.push_back(op);
      });

      if (deadNodes.empty())
        break;

      for (Operation *op : deadNodes) {
        if (!isUnusedAllowedPureNode(op))
          continue;
        op->erase();
        ++eliminated;
      }
    }

    const std::size_t irSizeAfter = computeModuleIRTextLength(module);
    const int64_t irDelta = static_cast<int64_t>(irSizeAfter) -
                            static_cast<int64_t>(irSizeBefore);
    writePassStats(module, getArgument(), eliminated, irDelta);
  }
};

} // namespace

std::unique_ptr<Pass> createPureDeadNodeEliminationPass() {
  return std::make_unique<PureDeadNodeEliminationPass>();
}

} // namespace mlir::ais
