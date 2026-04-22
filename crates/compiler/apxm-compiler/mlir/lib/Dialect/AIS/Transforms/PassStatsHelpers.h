// PassStatsHelpers.h
//
// Phase B Task 7 helper. Each AIS transform pass writes two module-level
// IntegerAttrs the apxm_module_drain_pass_stats CAPI then drains:
//
//   <pass-argument>_fired_count    : how many rewrites the pass committed
//   <pass-argument>_ir_size_delta  : signed change in printed IR length
//
// The Rust side reads + erases these attrs after each pass run and folds
// them into PassMetrics. Keeping the writer in one helper makes adding a
// new pass a one-line affair on the C++ side.
#pragma once

#include "mlir/IR/BuiltinOps.h"
#include "llvm/ADT/StringRef.h"

#include <cstddef>
#include <cstdint>

namespace mlir::ais {

// Print the module to a string and return its length. Used to bracket each
// pass with before/after measurements; the difference is the ir_size_delta.
std::size_t computeModuleIRTextLength(ModuleOp module);

// Stamp the two stats attrs onto the module. `passArgument` should be the
// pass's `getArgument()` value (e.g. "fuse-ask-ops"); the suffixes match
// what apxm_module_drain_pass_stats looks for.
void writePassStats(ModuleOp module,
                    llvm::StringRef passArgument,
                    std::uint64_t firedCount,
                    std::int64_t irSizeDelta);

}  // namespace mlir::ais
