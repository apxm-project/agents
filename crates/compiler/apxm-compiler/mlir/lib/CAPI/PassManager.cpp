/**
 * @file  PassManager.cpp
 * @brief Imperative pass pipeline builder for the C API.
 *
 * Creates an `mlir::PassManager`, exposes `addPassByName` for textual
 * pipelines, and provides typed helpers (`apxm_pass_manager_add_*`) for
 * hosts that prefer compile-time safety.  The manager is independent of
 * the C++ static registry: passes are looked up in the AIS library only.
 */

#include "ais/CAPI/PassManager.h"
#include "ais/CAPI/Module.h"
#include "ais/Dialect/AIS/Transforms/Passes.h"
#include "mlir/IR/BuiltinAttributes.h"
#include "mlir/Transforms/Passes.h"
#include "mlir/Pass/PassInstrumentation.h"
#include "llvm/ADT/SmallPtrSet.h"
#include "llvm/Support/FileSystem.h"
#include "llvm/Support/FormatVariadic.h"
#include "llvm/Support/Path.h"
#include "llvm/Support/raw_ostream.h"
#include <atomic>
#include <cstdlib>
#include <string>

extern "C" {

namespace {

struct ApXmIrPrinter final : public mlir::PassInstrumentation {
  explicit ApXmIrPrinter(std::string dirPath)
      : dir(std::move(dirPath)) {}

  void runAfterPass(mlir::Pass *pass, mlir::Operation *op) override {
    if (!op || dir.empty()) {
      return;
    }

    llvm::SmallString<256> path(dir);
    llvm::sys::path::append(
        path, llvm::formatv("{0}_{1}.mlir", counter.fetch_add(1),
                            pass ? pass->getName() : "unknown"));

    std::error_code ec;
    llvm::raw_fd_ostream os(path, ec, llvm::sys::fs::OF_Text);
    if (ec) {
      return;
    }
    op->print(os);
    os << "\n";
  }

  std::string dir;
  std::atomic<uint64_t> counter{0};
};

} // namespace

static void configureIrPrinting(mlir::PassManager &pm) {
  const char *printIrTrace = std::getenv("APXM_PRINT_IR_TRACE");
  const char *printIr = std::getenv("APXM_PRINT_IR");
  const char *printIrAfterAll = std::getenv("MLIR_PRINT_IR_AFTER_ALL");
  const char *printIrDir = std::getenv("APXM_PRINT_IR_DIR");
  if (printIrTrace && std::strcmp(printIrTrace, "0") != 0) {
    llvm::errs() << "[apxm] configureIrPrinting: dir="
                 << (printIrDir ? printIrDir : "<unset>")
                 << " print=" << (printIr ? printIr : "<unset>")
                 << " after_all=" << (printIrAfterAll ? printIrAfterAll : "<unset>")
                 << "\n";
  }
  if (printIrDir && std::strcmp(printIrDir, "0") != 0) {
    llvm::sys::fs::create_directories(printIrDir);
    pm.addInstrumentation(std::make_unique<ApXmIrPrinter>(printIrDir));
    return;
  }
  if ((printIr && std::strcmp(printIr, "0") != 0) ||
      (printIrAfterAll && std::strcmp(printIrAfterAll, "0") != 0)) {
    pm.enableIRPrinting(
        /*shouldPrintBeforePass=*/[](mlir::Pass *, mlir::Operation *) { return false; },
        /*shouldPrintAfterPass=*/[](mlir::Pass *, mlir::Operation *) { return true; },
        /*printModuleScope=*/true,
        /*printAfterOnlyOnChange=*/false,
        /*printAfterOnlyOnFailure=*/false);
  }
}

ApxmPassManager* apxm_pass_manager_create(ApxmCompilerContext* ctx) {
  if (!ctx) return nullptr;
  auto *pm = new (std::nothrow) ApxmPassManager(ctx);
  if (!pm) return nullptr;
  configureIrPrinting(*pm->pass_manager);
  return pm;
}

void apxm_pass_manager_destroy(ApxmPassManager* pm) {
  delete pm;
}

void apxm_pass_manager_clear(ApxmPassManager* pm) {
  if (!pm) return;

  pm->pass_manager = std::make_unique<mlir::PassManager>(pm->context->mlir_context.get());
  pm->registered_passes.clear();
  configureIrPrinting(*pm->pass_manager);
}

bool apxm_pass_manager_run(ApxmPassManager* pm, ApxmModule* module) {
  if (!pm || !module || !module->module) {
    return false;
  }
  return mlir::succeeded(pm->pass_manager->run(*module->module));
}

void apxm_pass_manager_add_inline(ApxmPassManager* pm);

bool apxm_pass_manager_has_pass(ApxmPassManager* pm, const char* pass_name) {
  if (!pm || !pass_name) return false;

  // Simple implementation - in real system would use pass registry
  static const char* known_passes[] = {
    "normalize", "build-prompt", "fuse-ask-ops", "condense-ops",
    "scheduling", "canonicalizer", "cse", "symbol-dce", "inline",
    "unconsumed-value-warning"
  };

  for (auto name : known_passes) {
    if (strcmp(name, pass_name) == 0) {
      return true;
    }
  }
  return false;
}

bool apxm_pass_manager_add_pass_by_name(ApxmPassManager* pm, const char* pass_name) {
  if (!pm || !pass_name) return false;

  // Use generated dispatch from Rust pass definitions
  llvm::StringRef name(pass_name);
  #include "ais/CAPI/PassDispatch.inc"

  return false;  // Unknown pass
}

// Analysis Passes
void apxm_pass_manager_add_unconsumed_value_warning(ApxmPassManager* pm) {
  if (pm) pm->pass_manager->addPass(mlir::ais::createUnconsumedValueWarningPass());
}

// Transform Passes
void apxm_pass_manager_add_normalize(ApxmPassManager* pm) {
  if (pm) pm->pass_manager->addPass(mlir::ais::createNormalizeAgentGraphPass());
}

void apxm_pass_manager_add_build_prompt(ApxmPassManager* pm) {
  if (pm) pm->pass_manager->addPass(mlir::ais::createBuildPromptPass());
}

void apxm_pass_manager_add_fuse_ask_ops(ApxmPassManager* pm) {
  if (pm) pm->pass_manager->addPass(mlir::ais::createFuseAskOpsPass());
}

void apxm_pass_manager_add_condense_ops(ApxmPassManager* pm) {
  if (pm) pm->pass_manager->addPass(mlir::ais::createCondenseOpsPass());
}

void apxm_pass_manager_add_scheduling(ApxmPassManager* pm) {
  if (pm) pm->pass_manager->addPass(mlir::ais::createCapabilitySchedulingPass());
}

// Optimization Passes
void apxm_pass_manager_add_canonicalizer(ApxmPassManager* pm) {
  if (pm) pm->pass_manager->addPass(mlir::createCanonicalizerPass());
}

void apxm_pass_manager_add_cse(ApxmPassManager* pm) {
  if (pm) pm->pass_manager->addPass(mlir::createCSEPass());
}

void apxm_pass_manager_add_symbol_dce(ApxmPassManager* pm) {
  if (pm) pm->pass_manager->addPass(mlir::createSymbolDCEPass());
}

void apxm_pass_manager_add_inline(ApxmPassManager* pm) {
  if (pm) pm->pass_manager->addPass(mlir::createInlinerPass());
}

int apxm_module_drain_pass_stats(ApxmModule* module,
                                 const char* pass_name,
                                 int64_t* fired_count_out,
                                 int64_t* ir_size_delta_out) {
  if (!module || !module->module || !pass_name || !fired_count_out ||
      !ir_size_delta_out) {
    return 1;
  }
  mlir::ModuleOp moduleOp = *module->module;
  auto drain = [&](const std::string& key, int64_t* out) {
    *out = 0;
    if (auto attr = moduleOp->getAttrOfType<mlir::IntegerAttr>(key)) {
      *out = attr.getInt();
      moduleOp->removeAttr(key);
    }
  };
  // Attribute keys are dialect-prefixed (`ais.`) on the write side; see
  // mlir/lib/Dialect/AIS/Transforms/PassStatsHelpers.cpp.
  drain(std::string("ais.") + pass_name + "_fired_count", fired_count_out);
  drain(std::string("ais.") + pass_name + "_ir_size_delta", ir_size_delta_out);
  return 0;
}

int apxm_module_strip_all_pass_stats(ApxmModule* module) {
  if (!module || !module->module) {
    return 1;
  }
  mlir::ModuleOp moduleOp = *module->module;
  llvm::SmallVector<llvm::StringRef, 16> toRemove;
  for (mlir::NamedAttribute attr : moduleOp->getAttrs()) {
    llvm::StringRef name = attr.getName().getValue();
    if (!name.starts_with("ais.")) continue;
    if (name.ends_with("_fired_count") || name.ends_with("_ir_size_delta")) {
      toRemove.push_back(name);
    }
  }
  for (llvm::StringRef name : toRemove) {
    moduleOp->removeAttr(name);
  }
  return 0;
}

int apxm_module_total_template_tokens(ApxmModule* module, uint64_t* total_out) {
  if (!module || !module->module || !total_out) {
    return 1;
  }
  mlir::ModuleOp moduleOp = *module->module;
  uint64_t total = 0;
  // Walk every op (including the module itself) and sum the
  // `ais.est_template_tokens` IntegerAttr where present. Ops that lack the
  // attribute contribute 0. Saturating-add to guard against pathological
  // overflow on very large graphs.
  moduleOp->walk([&](mlir::Operation* op) {
    if (auto attr = op->getAttrOfType<mlir::IntegerAttr>(
            "ais.est_template_tokens")) {
      uint64_t v = attr.getValue().getZExtValue();
      if (total > UINT64_MAX - v) {
        total = UINT64_MAX;
      } else {
        total += v;
      }
    }
  });
  *total_out = total;
  return 0;
}

} // extern "C"
