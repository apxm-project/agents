/**
 * @file  SchemaNarrowing.cpp
 * @brief Retains output schemas until specialization has typed legality proof.
 *
 * Schema specialization changes the request contract sent to a model. The
 * current AIS IR does not carry either a typed downstream field-use set or
 * selected-backend structured-output support into this pass, so it cannot
 * prove that requesting fewer fields preserves behavior.
 *
 * The pass is intentionally explicit-only and diagnostic. It retains every
 * output_schema attribute and emits a remark explaining why specialization was
 * skipped. A future rewrite must require both proofs before changing a schema.
 *
 * Before and after are identical without those proofs:
 *   %result = ais.ask "Extract a summary." {
 *     output_schema = {summary = "string", detail = "string"}
 *   } : !ais.token
 *
 * The schema remains unchanged even if `%result` has no uses. An unused result
 * does not prove that a runtime backend may omit structured-output validation.
 */

#include "ais/Dialect/AIS/Transforms/Passes.h"

#include "ais/Common/Constants.h"
#include "PassStatsHelpers.h"

#include "ais/Dialect/AIS/IR/AISOps.h"
#include "ais/Dialect/AIS/Support/AISDebug.h"

namespace mlir::ais {
#define GEN_PASS_DEF_SCHEMANARROWING
#include "ais/Dialect/AIS/Transforms/Passes.h.inc"

namespace {

APXM_AIS_DEBUG_SETUP(schema_narrowing)

struct SchemaNarrowingPass : impl::SchemaNarrowingBase<SchemaNarrowingPass> {
  using SchemaNarrowingBase::SchemaNarrowingBase;

  void runOnOperation() override {
    APXM_AIS_DEBUG_HEADER(SchemaNarrowing);
    ModuleOp module = getOperation();
    unsigned retained = 0;
    module.walk([&](Operation *op) {
      if (!op->hasAttr(apxm::constants::attrs::OUTPUT_SCHEMA)) {
        return;
      }
      ++retained;
      op->emitRemark()
          << "schema specialization skipped: output_schema is retained until "
             "typed downstream field-use facts and selected-backend "
             "structured-output support are available";
    });

    if (retained == 0) {
      APXM_AIS_DEBUG("No output schemas require guarded specialization");
    } else {
      APXM_AIS_INFO("Retained " << retained
                    << " output schema(s) without typed legality proof");
    }

    // This diagnostic pass never rewrites the module. The pass statistics
    // therefore report zero fired rewrites and zero IR-size delta.
    writePassStats(module, getArgument(), 0, 0);

    APXM_AIS_DEBUG_FOOTER(SchemaNarrowing);
  }
};

} // namespace

std::unique_ptr<Pass> createSchemaNarrowingPass() {
  return std::make_unique<SchemaNarrowingPass>();
}

} // namespace mlir::ais
