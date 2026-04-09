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
  unsigned criticalPathLength = 0;
};

/// Compute the longest path from each operation to a sink node (operation with no users).
/// This gives us the "depth" of each operation in the DAG.
static PriorityAnalysis analyzeDag(func::FuncOp func) {
  PriorityAnalysis analysis;

  // Collect all AIS operations
  llvm::SmallVector<Operation*> ops;
  func.walk([&](Operation* op) {
    // Skip func.return and other non-AIS ops
    if (!op->getDialect() || op->getDialect()->getNamespace() != "ais")
      return;
    ops.push_back(op);
  });

  // Assign sequential IDs matching ArtifactEmitter walk order
  unsigned nextId = 0;
  for (Operation* op : ops) {
    analysis.opToId[op] = nextId++;
  }

  // Compute fan-out (number of consumers for each operation)
  for (Operation* op : ops) {
    analysis.fanOut[op] = 0;
  }

  for (Operation* op : ops) {
    for (Value operand : op->getOperands()) {
      if (auto* defOp = operand.getDefiningOp()) {
        if (defOp->getDialect() && defOp->getDialect()->getNamespace() == "ais") {
          analysis.fanOut[defOp]++;
        }
      }
    }
  }

  // Compute longest path using reverse topological order (bottom-up)
  // Start with sink nodes (operations with no users or whose users are outside AIS)
  llvm::SmallVector<Operation*> worklist;
  llvm::DenseMap<Operation*, bool> visited;

  for (Operation* op : ops) {
    unsigned aisUsers = 0;
    for (Operation* user : op->getUsers()) {
      if (user->getDialect() && user->getDialect()->getNamespace() == "ais") {
        aisUsers++;
      }
    }
    if (aisUsers == 0) {
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
        if (predOp->getDialect() && predOp->getDialect()->getNamespace() == "ais") {
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
      func.walk([&](Operation* op) {
        if (!op->getDialect() || op->getDialect()->getNamespace() != "ais")
          return;

        unsigned longestPath = analysis.longestPath.lookup(op);
        unsigned fanOut = analysis.fanOut.lookup(op);

        // Priority assignment strategy (matches Rust parallelism_analysis):
        // - Critical path nodes (depth near critical_path_length) → 90 (Critical)
        // - High fan-out nodes (3+ consumers) → 70 (High)
        // - Default → 30 (Normal)

        bool isOnCriticalPath = longestPath >= (analysis.criticalPathLength > 0
                                                 ? analysis.criticalPathLength - 1
                                                 : 0);

        unsigned priority;
        if (isOnCriticalPath) {
          priority = 90;
          stats.criticalPrio++;
        } else if (fanOut >= 3) {
          priority = 70;
          stats.highPrio++;
        } else {
          priority = 30;
          stats.normalPrio++;
        }

        OpBuilder builder(op);
        op->setAttr("priority", builder.getI32IntegerAttr(static_cast<int32_t>(priority)));

        // Emit downstream_nodes: collect IDs of AIS ops that consume this op's results
        llvm::SmallVector<Attribute> downstreamIds;
        for (Operation* user : op->getUsers()) {
          if (user->getDialect() && user->getDialect()->getNamespace() == "ais") {
            auto it = analysis.opToId.find(user);
            if (it != analysis.opToId.end()) {
              downstreamIds.push_back(
                  builder.getI32IntegerAttr(static_cast<int32_t>(it->second)));
            }
          }
        }
        op->setAttr(apxm::constants::attrs::DOWNSTREAM_NODES,
                     builder.getArrayAttr(downstreamIds));

        APXM_AIS_DEBUG("  " << op->getName() << ": priority=" << priority
                        << " (longest_path=" << longestPath
                        << ", fan_out=" << fanOut
                        << ", downstream=" << downstreamIds.size() << ")");
      });
    }

    APXM_AIS_INFO("Assigned priorities: critical=" << stats.criticalPrio
                  << ", high=" << stats.highPrio
                  << ", normal=" << stats.normalPrio);
    APXM_AIS_DEBUG_FOOTER(AssignPriority);
  }
};

}  // namespace

std::unique_ptr<Pass> createAssignPriorityPass() {
  return std::make_unique<AssignPriorityPass>();
}

}  // namespace mlir::ais
