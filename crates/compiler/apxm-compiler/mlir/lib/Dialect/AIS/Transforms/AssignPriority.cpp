/**
 * @file  AssignPriority.cpp
 * @brief Assigns execution priority based on critical path analysis.
 *
 * This pass performs DAG analysis to compute the critical path and assigns
 * priority attributes to each AIS operation for the runtime scheduler.
 *
 * Priority levels:
 *   - 90 (Critical): Operations on or near the critical path
 *   - 70 (High): Operations with high fan-out (3+ consumers)
 *   - 30 (Normal): All other operations
 *
 * The priority attribute is stored as an IntegerAttr on each operation and
 * later extracted by the ArtifactEmitter into node.metadata.priority.
 */

#include "ais/Dialect/AIS/Transforms/Passes.h"

#include "ais/Common/Constants.h"
#include "PassStatsHelpers.h"
#include "ais/Dialect/AIS/IR/AISOps.h"
#include "ais/Dialect/AIS/Support/AISDebug.h"

#include "mlir/Dialect/Func/IR/FuncOps.h"
#include "mlir/IR/Builders.h"
#include "llvm/ADT/DenseMap.h"
#include "llvm/ADT/SmallVector.h"

#include <algorithm>

namespace mlir::ais {
#define GEN_PASS_DEF_ASSIGNPRIORITY
#include "ais/Dialect/AIS/Transforms/Passes.h.inc"

namespace {

APXM_AIS_DEBUG_SETUP(assign_priority)

struct PriorityAnalysis {
  llvm::DenseMap<Operation*, unsigned> longestPath;
  llvm::DenseMap<Operation*, unsigned> fanOut;
  llvm::DenseMap<Operation*, unsigned> opToId;
  llvm::DenseMap<Operation*, llvm::SmallVector<Operation*, 8>> downstream;
  unsigned criticalPathLength = 0;
};

static bool isAisOperation(Operation *op) {
  return op && op->getDialect() && op->getDialect()->getNamespace() == "ais";
}

static bool isArtifactOperation(Operation *op) {
  return isAisOperation(op) || isa<func::ReturnOp>(op);
}

/// Compute the longest path from each operation to a sink node (operation with no users).
/// This gives us the "depth" of each operation in the DAG.
static PriorityAnalysis analyzeDag(func::FuncOp func) {
  PriorityAnalysis analysis;

  // Collect all AIS operations
  llvm::SmallVector<Operation*> ops;
  for (Block &block : func) {
    for (Operation &op : block) {
      if (!isArtifactOperation(&op))
        continue;
      ops.push_back(&op);
    }
  }

  // Assign 1-based IDs matching ArtifactEmitter node IDs and walk order.
  unsigned nextId = 1;
  for (Operation* op : ops) {
    analysis.opToId[op] = nextId++;
    analysis.downstream[op] = {};
  }

  // Compute downstream consumers by scanning operands, matching ArtifactEmitter
  // edge construction.
  for (Operation* consumer : ops) {
    for (Value operand : consumer->getOperands()) {
      Operation* producer = operand.getDefiningOp();
      if (!isArtifactOperation(producer))
        continue;

      auto &consumers = analysis.downstream[producer];
      if (std::find(consumers.begin(), consumers.end(), consumer) == consumers.end()) {
        consumers.push_back(consumer);
      }
    }
  }

  // Compute fan-out (number of unique consumers for each operation)
  for (Operation* op : ops) {
    auto it = analysis.downstream.find(op);
    analysis.fanOut[op] = it == analysis.downstream.end() ? 0 : it->second.size();
  }

  // Compute longest path using reverse topological order (bottom-up)
  // Start with sink nodes (operations with no users or whose users are outside AIS)
  llvm::SmallVector<Operation*> worklist;
  llvm::DenseMap<Operation*, bool> visited;

  for (Operation* op : ops) {
    auto it = analysis.downstream.find(op);
    bool hasDownstream = it != analysis.downstream.end() && !it->second.empty();
    if (!hasDownstream) {
      worklist.push_back(op);
      analysis.longestPath[op] = 1;
      visited[op] = true;
    }
  }

  // Process worklist: for each operation, propagate longest path to predecessors
  while (!worklist.empty()) {
    Operation* current = worklist.pop_back_val();
    unsigned currentDepth = analysis.longestPath[current];
    analysis.criticalPathLength = std::max(analysis.criticalPathLength, currentDepth);

    // Update all predecessors
    for (Value operand : current->getOperands()) {
      if (auto* predOp = operand.getDefiningOp()) {
        if (isArtifactOperation(predOp)) {
          unsigned newDepth = currentDepth + 1;
          if (newDepth > analysis.longestPath[predOp]) {
            analysis.longestPath[predOp] = newDepth;
            if (!visited[predOp]) {
              visited[predOp] = true;
              worklist.push_back(predOp);
            } else {
              // Re-add to worklist to propagate updated depth
              worklist.push_back(predOp);
            }
          }
        }
      }
    }
  }

  return analysis;
}

struct AssignPriorityPass : impl::AssignPriorityBase<AssignPriorityPass> {
  void runOnOperation() override {
    APXM_AIS_DEBUG_HEADER(AssignPriority);
    ModuleOp module = getOperation();
    const std::size_t irSizeBefore = computeModuleIRTextLength(module);

    struct Statistics {
      unsigned criticalPrio = 0;
      unsigned highPrio = 0;
      unsigned normalPrio = 0;
    } stats;

    // Analyze each function in the module
    for (auto func : module.getOps<func::FuncOp>()) {
      auto analysis = analyzeDag(func);

      APXM_AIS_DEBUG("Function: " << func.getName()
                      << ", Critical path length: " << analysis.criticalPathLength);

      // Assign priorities to all AIS operations
      for (Block &block : func) {
        for (Operation &op : block) {
          if (!isAisOperation(&op))
            continue;

          Operation *opPtr = &op;
          unsigned longestPath = analysis.longestPath.lookup(opPtr);
          unsigned fanOut = analysis.fanOut.lookup(opPtr);

          // Priority assignment strategy (matches Rust parallelism_analysis):
          // - Critical path nodes (depth == critical_path_length) → Critical
          // - High fan-out nodes (FAN_OUT_THRESHOLD+ consumers) → High
          // - Default → Normal

          bool isOnCriticalPath = (longestPath == analysis.criticalPathLength);

          unsigned priority;
          if (isOnCriticalPath) {
            priority = apxm::constants::priority::CRITICAL;
            stats.criticalPrio++;
          } else if (fanOut >= apxm::constants::priority::FAN_OUT_THRESHOLD) {
            priority = apxm::constants::priority::HIGH;
            stats.highPrio++;
          } else {
            priority = apxm::constants::priority::NORMAL;
            stats.normalPrio++;
          }

          OpBuilder builder(opPtr);
          opPtr->setAttr(apxm::constants::attrs::PRIORITY,
                         builder.getI32IntegerAttr(static_cast<int32_t>(priority)));

          // Emit downstream_nodes: collect IDs of AIS ops that consume this op's results
          llvm::SmallVector<unsigned> downstreamNodeIds;
          if (auto downstreamIt = analysis.downstream.find(opPtr);
              downstreamIt != analysis.downstream.end()) {
            for (Operation* user : downstreamIt->second) {
              auto it = analysis.opToId.find(user);
              if (it != analysis.opToId.end()) {
                downstreamNodeIds.push_back(it->second);
              }
            }
          }
          std::sort(downstreamNodeIds.begin(), downstreamNodeIds.end());
          llvm::SmallVector<Attribute> downstreamIds;
          downstreamIds.reserve(downstreamNodeIds.size());
          for (unsigned downstreamNodeId : downstreamNodeIds) {
            downstreamIds.push_back(
                builder.getI32IntegerAttr(static_cast<int32_t>(downstreamNodeId)));
          }
          opPtr->setAttr(apxm::constants::attrs::DOWNSTREAM_NODES,
                         builder.getArrayAttr(downstreamIds));

          APXM_AIS_DEBUG("  " << opPtr->getName() << ": priority=" << priority
                          << " (longest_path=" << longestPath
                          << ", fan_out=" << fanOut
                          << ", downstream=" << downstreamIds.size() << ")");
        }
      }
    }

    APXM_AIS_INFO("Assigned priorities: critical=" << stats.criticalPrio
                  << ", high=" << stats.highPrio
                  << ", normal=" << stats.normalPrio);

    // Phase B Task 7: per-pass stats drained by apxm_module_drain_pass_stats.
    // fired_count = total ops that received a priority annotation.
    const uint64_t totalAnnotated = static_cast<uint64_t>(stats.criticalPrio)
                                  + stats.highPrio + stats.normalPrio;
    const std::size_t irSizeAfter = computeModuleIRTextLength(module);
    const int64_t irDelta = static_cast<int64_t>(irSizeAfter)
                          - static_cast<int64_t>(irSizeBefore);
    writePassStats(module, getArgument(), totalAnnotated, irDelta);

    APXM_AIS_DEBUG_FOOTER(AssignPriority);
  }
};

}  // namespace

std::unique_ptr<Pass> createAssignPriorityPass() {
  return std::make_unique<AssignPriorityPass>();
}

}  // namespace mlir::ais
