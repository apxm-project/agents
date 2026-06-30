// PassStatsHelpers.cpp — see header for contract.
#include "PassStatsHelpers.h"

#include "ais/Common/Constants.h"
#include "mlir/IR/Builders.h"
#include "mlir/IR/BuiltinAttributes.h"
#include "mlir/IR/BuiltinTypes.h"
#include "llvm/Support/raw_ostream.h"

#include <string>

namespace mlir::ais {

std::size_t computeModuleIRTextLength(ModuleOp module) {
  std::string buf;
  llvm::raw_string_ostream os(buf);
  module->print(os);
  os.flush();
  return buf.size();
}

void writePassStats(ModuleOp module,
                    llvm::StringRef passArgument,
                    std::uint64_t firedCount,
                    std::int64_t irSizeDelta) {
  // ModuleOp requires dialect-prefixed attribute names.
  auto* ctx = module.getContext();
  auto i64 = IntegerType::get(ctx, 64);
  std::string firedKey =
      (apxm::constants::attrs::DIALECT_ATTR_PREFIX + passArgument +
       apxm::constants::attrs::PASS_STATS_FIRED_SUFFIX)
          .str();
  std::string deltaKey =
      (apxm::constants::attrs::DIALECT_ATTR_PREFIX + passArgument +
       apxm::constants::attrs::PASS_STATS_IR_SIZE_DELTA_SUFFIX)
          .str();
  module->setAttr(firedKey,
                  IntegerAttr::get(i64, static_cast<int64_t>(firedCount)));
  module->setAttr(deltaKey, IntegerAttr::get(i64, irSizeDelta));
}

}  // namespace mlir::ais
