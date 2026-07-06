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
#include <optional>

namespace mlir::ais {
#define GEN_PASS_DEF_ASSIGNPRIORITY
#include "ais/Dialect/AIS/Transforms/Passes.h.inc"

namespace {

APXM_AIS_DEBUG_SETUP(assign_priority)

struct PriorityAnalysis {
  llvm::DenseMap<Operation*, unsigned> longestPath;
  llvm::DenseMap<Operation*, unsigned> fanOut;
  llvm::DenseMap<Operation*, unsigned> stageIndex;
  llvm::DenseMap<Operation*, unsigned> opToId;
  llvm::DenseMap<Operation*, llvm::SmallVector<Operation*, 8>> downstream;
  llvm::DenseMap<Operation*, bool> criticalPath;
  unsigned criticalPathLength = 0;
};

static bool isAisOperation(Operation *op) {
  return op && op->getDialect() && op->getDialect()->getNamespace() == "ais";
}

static bool isArtifactOperation(Operation *op) {
  // `op` may be null when it comes from `Value::getDefiningOp()` on a block
  // argument (e.g. an AIS op that takes a function parameter directly as an
  // operand). `isa<>` requires a non-null pointer, so the guard is mandatory:
  // without it `isa<func::ReturnOp>(nullptr)` dereferences null and segfaults
  // the whole compiler (and any server compiling untrusted AIR at O1+).
  return op && (isAisOperation(op) || isa<func::ReturnOp>(op));
}

static std::optional<llvm::StringRef> graphLatencyClass(Operation *op) {
  auto latencyAttr =
      op->getAttrOfType<AISLatencyAttr>(apxm::constants::attrs::LATENCY);
  if (!latencyAttr)
    return std::nullopt;

  switch (latencyAttr.getValue()) {
  case AISLatencyKind::low:
    return apxm::constants::graph_metrics::LATENCY_SHORT;
  case AISLatencyKind::medium:
    return apxm::constants::graph_metrics::LATENCY_MEDIUM;
  case AISLatencyKind::high:
    return apxm::constants::graph_metrics::LATENCY_LONG;
  }
  return std::nullopt;
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

  // Compute a 0-based stage index from graph sources to each operation.
  for (Operation* op : ops) {
    unsigned stage = 0;
    for (Value operand : op->getOperands()) {
      Operation* producer = operand.getDefiningOp();
      if (!isArtifactOperation(producer))
        continue;

      stage = std::max(stage, analysis.stageIndex.lookup(producer) + 1);
    }
    analysis.stageIndex[op] = stage;
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

  llvm::SmallVector<Operation*> criticalWorklist;
  for (Operation* op : ops) {
    if (analysis.longestPath.lookup(op) == analysis.criticalPathLength) {
      criticalWorklist.push_back(op);
    }
  }

  while (!criticalWorklist.empty()) {
    Operation* current = criticalWorklist.pop_back_val();
    if (analysis.criticalPath.lookup(current))
      continue;

    analysis.criticalPath[current] = true;
    const unsigned currentDepth = analysis.longestPath.lookup(current);
    auto downstreamIt = analysis.downstream.find(current);
    if (downstreamIt == analysis.downstream.end())
      continue;

    for (Operation* child : downstreamIt->second) {
      if (analysis.longestPath.lookup(child) + 1 == currentDepth) {
        criticalWorklist.push_back(child);
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
          unsigned stageIndex = analysis.stageIndex.lookup(opPtr);

          // Priority assignment strategy (matches Rust parallelism_analysis):
          // - Critical path nodes (members of at least one maximum-depth path)
          //   → Critical
          // - High fan-out nodes (FAN_OUT_THRESHOLD+ consumers) → High
          // - Default → Normal

          bool isOnCriticalPath = analysis.criticalPath.lookup(opPtr);

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
          opPtr->setAttr(apxm::constants::attrs::FANOUT_COUNT,
                         builder.getI32IntegerAttr(static_cast<int32_t>(fanOut)));
          opPtr->setAttr(
              apxm::constants::attrs::REMAINING_PATH_LEN,
              builder.getI32IntegerAttr(static_cast<int32_t>(longestPath)));
          opPtr->setAttr(apxm::constants::attrs::STAGE_INDEX,
                         builder.getI32IntegerAttr(static_cast<int32_t>(stageIndex)));
          if (auto latencyClass = graphLatencyClass(opPtr)) {
            opPtr->setAttr(apxm::constants::attrs::LATENCY_CLASS,
                           builder.getStringAttr(*latencyClass));
          }
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
                          << ", stage_index=" << stageIndex
                          << ", downstream=" << downstreamIds.size() << ")");
        }
      }
    }

    APXM_AIS_INFO("Assigned priorities: critical=" << stats.criticalPrio
                  << ", high=" << stats.highPrio
                  << ", normal=" << stats.normalPrio);

    // Per-pass stats are drained by apxm_module_drain_pass_stats.
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
