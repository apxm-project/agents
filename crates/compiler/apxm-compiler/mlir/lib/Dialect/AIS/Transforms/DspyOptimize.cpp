/**
 * @file  DspyOptimize.cpp
 * @brief Optimize prompt templates using DSPy (Stanford NLP).
 *
 * This pass invokes python3 -m apxm_dspy as a subprocess to run DSPy's
 * prompt optimizers (MIPROv2, BootstrapFewShot, COPRO) on template strings.
 * It is a no-op when no training data is available.
 *
 * Placement: immediately after build-prompt (which establishes {0} placeholders).
 */

#include "ais/Dialect/AIS/Transforms/Passes.h"

#include "ais/Common/Constants.h"
#include "PassStatsHelpers.h"
#include "ais/Dialect/AIS/IR/AISOps.h"
#include "ais/Dialect/AIS/Support/AISDebug.h"

#include "mlir/IR/Builders.h"
#include "llvm/ADT/TypeSwitch.h"
#include "llvm/Support/FileSystem.h"
#include "llvm/Support/JSON.h"
#include "llvm/Support/MemoryBuffer.h"
#include "llvm/Support/Path.h"
#include "llvm/Support/Program.h"
#include "llvm/Support/raw_ostream.h"

namespace mlir::ais {
#define GEN_PASS_DEF_DSPYOPTIMIZE
#include "ais/Dialect/AIS/Transforms/Passes.h.inc"

namespace {

APXM_AIS_DEBUG_SETUP(dspy_optimize)

struct DspyOptimizePass : impl::DspyOptimizeBase<DspyOptimizePass> {
  using DspyOptimizeBase::DspyOptimizeBase;

  void runOnOperation() override {
    APXM_AIS_DEBUG_HEADER(DspyOptimize);
    ModuleOp module = getOperation();

    // Phase B Task 7: capture IR size at entry; fire writePassStats on every
    // exit path via an RAII guard so early returns still publish stats.
    unsigned optimized = 0;
    const std::size_t irSizeBefore = computeModuleIRTextLength(module);
    struct StatsGuard {
      ModuleOp module;
      llvm::StringRef passArg;
      unsigned &counter;
      std::size_t before;
      ~StatsGuard() {
        const std::size_t after = computeModuleIRTextLength(module);
        const int64_t delta = static_cast<int64_t>(after)
                            - static_cast<int64_t>(before);
        writePassStats(module, passArg, counter, delta);
      }
    } statsGuard{module, getArgument(), optimized, irSizeBefore};

    // 1. Check for training data path (set by Rust layer as module attribute)
    auto trainingAttr =
        module->getAttrOfType<StringAttr>(apxm::constants::attrs::DSPY_TRAINING_DATA_PATH);
    if (!trainingAttr) {
      APXM_AIS_DEBUG("No training data path — dspy-optimize is a no-op");
      APXM_AIS_DEBUG_FOOTER(DspyOptimize);
      return;
    }

    // 2. Check for backend config (JSON string attribute)
    auto backendAttr =
        module->getAttrOfType<StringAttr>(apxm::constants::attrs::DSPY_BACKEND_JSON);
    auto optimizerAttr =
        module->getAttrOfType<StringAttr>(apxm::constants::attrs::DSPY_OPTIMIZER);
    auto autoAttr =
        module->getAttrOfType<StringAttr>(apxm::constants::attrs::DSPY_AUTO);
    auto metricAttr =
        module->getAttrOfType<StringAttr>(apxm::constants::attrs::DSPY_METRIC);

    // 3. Find python3
    auto pythonOrErr = llvm::sys::findProgramByName("python3");
    if (!pythonOrErr) {
      module->emitWarning("dspy-optimize: python3 not found in PATH, skipping");
      APXM_AIS_DEBUG_FOOTER(DspyOptimize);
      return;
    }
    std::string python = *pythonOrErr;

    // 4. Collect all LLM ops with non-empty templates
    struct OpInfo {
      Operation *op;
      std::string templateStr;
    };
    llvm::SmallVector<OpInfo> opsToOptimize;

    module.walk([&](Operation *op) {
      llvm::TypeSwitch<Operation *>(op)
          .Case<AskOp, ThinkOp, ReasonOp>([&](auto llmOp) {
            StringRef tmpl = llmOp.getTemplateStrAttr().getValue();
            if (!tmpl.empty()) {
              opsToOptimize.push_back({op, tmpl.str()});
            }
          });
    });

    if (opsToOptimize.empty()) {
      APXM_AIS_DEBUG("No LLM ops with templates to optimize");
      APXM_AIS_DEBUG_FOOTER(DspyOptimize);
      return;
    }

    APXM_AIS_INFO("Found " << opsToOptimize.size()
                            << " LLM ops to optimize with DSPy");

    // 5. Build batch request JSON
    llvm::json::Object requestObj;
    requestObj["training_data_path"] = trainingAttr.getValue().str();
    if (backendAttr)
      requestObj["backend_json"] = backendAttr.getValue().str();
    if (optimizerAttr)
      requestObj["optimizer"] = optimizerAttr.getValue().str();
    if (autoAttr)
      requestObj["auto"] = autoAttr.getValue().str();
    if (metricAttr)
      requestObj["metric"] = metricAttr.getValue().str();

    llvm::json::Array templatesArr;
    for (auto &info : opsToOptimize) {
      llvm::json::Object tmplObj;
      tmplObj["template_str"] = info.templateStr;
      templatesArr.push_back(std::move(tmplObj));
    }
    requestObj["templates"] = std::move(templatesArr);

    // 6. Write request to temp file
    llvm::SmallString<128> requestPath, responsePath;
    if (auto ec = llvm::sys::fs::createTemporaryFile("dspy-req", "json",
                                                      requestPath)) {
      module->emitWarning("dspy-optimize: failed to create temp file: " +
                          ec.message());
      return;
    }
    if (auto ec = llvm::sys::fs::createTemporaryFile("dspy-resp", "json",
                                                      responsePath)) {
      llvm::sys::fs::remove(requestPath);
      module->emitWarning("dspy-optimize: failed to create temp file: " +
                          ec.message());
      return;
    }

    {
      std::error_code EC;
      llvm::raw_fd_ostream reqFile(requestPath, EC);
      if (EC) {
        module->emitWarning("dspy-optimize: failed to write request: " +
                            EC.message());
        llvm::sys::fs::remove(requestPath);
        llvm::sys::fs::remove(responsePath);
        return;
      }
      reqFile << llvm::json::Value(std::move(requestObj));
    }

    // 7. Execute python3 -m apxm_dspy with file redirects
    std::optional<llvm::StringRef> redirects[] = {
        llvm::StringRef(requestPath),  // stdin
        llvm::StringRef(responsePath), // stdout
        std::nullopt                   // stderr (inherit for progress output)
    };

    llvm::SmallVector<llvm::StringRef> args = {python, "-m", "apxm_dspy"};
    std::string errMsg;
    int rc = llvm::sys::ExecuteAndWait(
        python, args,
        /*Env=*/std::nullopt,
        redirects,
        /*SecondsToWait=*/300, // 5 min timeout
        /*MemoryLimit=*/0,
        &errMsg);

    // 8. Clean up request file (response still needed)
    llvm::sys::fs::remove(requestPath);

    if (rc != 0) {
      std::string reason;
      if (rc == -1)
        reason = "execution failed: " + errMsg;
      else if (rc == -2)
        reason = "timed out after 300s";
      else
        reason = "exited with code " + std::to_string(rc);

      module->emitWarning("dspy-optimize: subprocess " + reason +
                          " — using original templates");
      llvm::sys::fs::remove(responsePath);
      APXM_AIS_DEBUG_FOOTER(DspyOptimize);
      return;
    }

    // 9. Read response
    auto bufOrErr = llvm::MemoryBuffer::getFile(responsePath);
    llvm::sys::fs::remove(responsePath);

    if (!bufOrErr) {
      module->emitWarning("dspy-optimize: failed to read response file");
      APXM_AIS_DEBUG_FOOTER(DspyOptimize);
      return;
    }

    auto responseJson = llvm::json::parse((*bufOrErr)->getBuffer());
    if (!responseJson) {
      module->emitWarning("dspy-optimize: invalid JSON response");
      APXM_AIS_DEBUG_FOOTER(DspyOptimize);
      return;
    }

    auto *respObj = responseJson->getAsObject();
    if (!respObj) {
      module->emitWarning("dspy-optimize: response is not a JSON object");
      APXM_AIS_DEBUG_FOOTER(DspyOptimize);
      return;
    }

    auto status = respObj->getString("status");
    if (!status || *status != "ok") {
      auto errorMsg = respObj->getString("error");
      std::string msg = errorMsg ? errorMsg->str() : "unknown error";
      module->emitWarning("dspy-optimize: " + msg);
      APXM_AIS_DEBUG_FOOTER(DspyOptimize);
      return;
    }

    // 10. Apply optimized templates
    OpBuilder builder(module.getContext());

    // Single template mode
    if (auto optimizedTmpl = respObj->getString("optimized_template")) {
      if (!opsToOptimize.empty()) {
        auto &info = opsToOptimize[0];
        llvm::TypeSwitch<Operation *>(info.op)
            .Case<AskOp, ThinkOp, ReasonOp>([&](auto llmOp) {
              llmOp.setTemplateStrAttr(
                  builder.getStringAttr(*optimizedTmpl));
              optimized++;
            });
      }
    }

    // Batch mode
    if (auto *resultsArr = respObj->getArray("results")) {
      for (size_t i = 0;
           i < resultsArr->size() && i < opsToOptimize.size(); ++i) {
        auto *resultObj = (*resultsArr)[i].getAsObject();
        if (!resultObj)
          continue;

        auto optTmpl = resultObj->getString("optimized_template");
        if (!optTmpl)
          continue;

        auto &info = opsToOptimize[i];
        llvm::TypeSwitch<Operation *>(info.op)
            .Case<AskOp, ThinkOp, ReasonOp>([&](auto llmOp) {
              llmOp.setTemplateStrAttr(builder.getStringAttr(*optTmpl));
              optimized++;
            });
      }
    }

    // 11. Set module attributes for diagnostics
    if (optimized > 0) {
      module->setAttr(
          apxm::constants::attrs::DSPY_OPTIMIZED,
          IntegerAttr::get(IntegerType::get(module.getContext(), 64),
                           optimized));
      APXM_AIS_INFO("Optimized " << optimized << " templates with DSPy");
    }

    APXM_AIS_DEBUG_FOOTER(DspyOptimize);
  }
};

} // namespace

std::unique_ptr<Pass> createDspyOptimizePass() {
  return std::make_unique<DspyOptimizePass>();
}

} // namespace mlir::ais
