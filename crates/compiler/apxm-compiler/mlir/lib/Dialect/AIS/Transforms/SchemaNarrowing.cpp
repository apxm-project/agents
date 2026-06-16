/**
 * @file  SchemaNarrowing.cpp
 * @brief Narrows output schemas based on actual usage patterns.
 *
 * When an output_schema attribute is set on a node, this pass propagates
 * schema constraints to downstream consumers and narrows overly broad schemas
 * based on how the output is actually used.
 *
 * Example: If a REASON operation specifies it returns {name, age, address}
 * but downstream operations only access 'name' and 'age', the schema can be
 * narrowed to {name, age}, reducing token overhead in the LLM response.
 *
 * Current implementation focuses on:
 * 1. Detecting operations with output_schema attributes
 * 2. Tracking which result values are actually consumed
 * 3. Narrowing schemas when entire outputs are unused
 *
 * Future enhancements could include:
 * - JSON path analysis to track field-level access
 * - Cross-function schema propagation
 * - Schema inference from downstream usage patterns
 */

#include "ais/Dialect/AIS/Transforms/Passes.h"

#include "ais/Common/Constants.h"
#include "PassStatsHelpers.h"

#include "ais/Dialect/AIS/IR/AISOps.h"
#include "ais/Dialect/AIS/Support/AISDebug.h"

#include "mlir/IR/Builders.h"
#include "llvm/ADT/DenseMap.h"
#include "llvm/ADT/DenseSet.h"

namespace mlir::ais {
#define GEN_PASS_DEF_SCHEMANARROWING
#include "ais/Dialect/AIS/Transforms/Passes.h.inc"

namespace {

APXM_AIS_DEBUG_SETUP(schema_narrowing)

/// Check if a value has any downstream operation.
static bool isValueConsumed(Value value) {
  return !value.use_empty();
}

/// Count the number of uses of a value
static unsigned countUses(Value value) {
  return std::distance(value.use_begin(), value.use_end());
}

struct SchemaNarrowingPass : impl::SchemaNarrowingBase<SchemaNarrowingPass> {
  using SchemaNarrowingBase::SchemaNarrowingBase;

  void runOnOperation() override {
    APXM_AIS_DEBUG_HEADER(SchemaNarrowing);
    ModuleOp module = getOperation();
    const std::size_t irSizeBefore = computeModuleIRTextLength(module);
    unsigned narrowed = 0;

    // Phase 1: Identify operations with output_schema attributes
    SmallVector<Operation*> opsWithSchema;
    module.walk([&](Operation *op) {
      if (op->hasAttr("output_schema")) {
        opsWithSchema.push_back(op);
        APXM_AIS_DEBUG("Found op with output_schema: " << op->getName());
      }
    });

    if (opsWithSchema.empty()) {
      APXM_AIS_DEBUG("No operations with output_schema found");
      writePassStats(module, getArgument(), 0, 0);
      APXM_AIS_DEBUG_FOOTER(SchemaNarrowing);
      return;
    }

    // Phase 2: Analyze usage of each operation's results
    for (Operation *op : opsWithSchema) {
      if (narrowSchema(op)) {
        narrowed++;
      }
    }

    if (narrowed > 0) {
      module->setAttr(apxm::constants::attrs::SCHEMAS_NARROWED,
                      IntegerAttr::get(IntegerType::get(module.getContext(), 64),
                                       narrowed));
      APXM_AIS_INFO("Narrowed " << narrowed << " output schemas");
    } else {
      APXM_AIS_DEBUG("No schemas were narrowed");
    }

    // Phase B Task 7: per-pass stats drained by apxm_module_drain_pass_stats.
    const std::size_t irSizeAfter = computeModuleIRTextLength(module);
    const int64_t irDelta = static_cast<int64_t>(irSizeAfter)
                          - static_cast<int64_t>(irSizeBefore);
    writePassStats(module, getArgument(), narrowed, irDelta);

    APXM_AIS_DEBUG_FOOTER(SchemaNarrowing);
  }

private:
  /// Attempt to narrow the output schema of an operation.
  /// Returns true if the schema was narrowed.
  bool narrowSchema(Operation *op) {
    auto schemaAttr = op->getAttr("output_schema");
    if (!schemaAttr) {
      return false;
    }

    // For now, we implement a simple optimization:
    // If the operation's result is never used, we can remove the output_schema
    // attribute entirely, signaling to the LLM that no structured output is needed.

    bool hasUnusedResults = false;
    unsigned totalResults = op->getNumResults();
    unsigned unusedResults = 0;

    for (auto result : op->getResults()) {
      if (!isValueConsumed(result)) {
        hasUnusedResults = true;
        unusedResults++;
        APXM_AIS_DEBUG("  Result " << result.getResultNumber() << " is unused");
      } else {
        unsigned uses = countUses(result);
        APXM_AIS_DEBUG("  Result " << result.getResultNumber() << " has "
                       << uses << " use(s)");
      }
    }

    // If ALL results are unused, remove the schema constraint entirely
    if (totalResults > 0 && unusedResults == totalResults) {
      APXM_AIS_INFO("Removing output_schema from " << op->getName()
                    << " (all results unused)");
      op->removeAttr("output_schema");
      return true;
    }

    // More sophisticated narrowing would go here:
    // - Parse the schema (JSON/dict structure)
    // - Track which fields are accessed downstream
    // - Build a narrowed schema with only used fields
    // - Update the attribute
    //
    // For now, we only handle the simple case of completely unused results.
    // This still provides value by eliminating unnecessary schema constraints.

    APXM_AIS_DEBUG("  Schema attribute retained (results are used)");
    return false;
  }
};

} // namespace

std::unique_ptr<Pass> createSchemaNarrowingPass() {
  return std::make_unique<SchemaNarrowingPass>();
}

} // namespace mlir::ais
