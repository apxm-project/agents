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

static std::string pythonStringLiteral(llvm::StringRef value) {
  std::string out = "\"";
  for (char c : value) {
    switch (c) {
    case '\\':
      out += "\\\\";
      break;
    case '"':
      out += "\\\"";
      break;
    case '\n':
      out += "\\n";
      break;
    case '\r':
      out += "\\r";
      break;
    case '\t':
      out += "\\t";
      break;
    default:
      out += c;
      break;
    }
  }
  out += "\"";
  return out;
}

static std::string dspyBootstrapCode() {
  llvm::SmallString<256> toolsPath;
#ifdef APXM_WORKSPACE_ROOT
  toolsPath = APXM_WORKSPACE_ROOT;
  llvm::sys::path::append(toolsPath, "tools");
#endif

  std::string code =
      "import importlib.util, runpy, sys\n"
      "tools_path = ";
  code += pythonStringLiteral(toolsPath);
  code +=
      "\n"
      "if importlib.util.find_spec('apxm_dspy') is None and tools_path:\n"
      "    sys.path.insert(0, tools_path)\n"
      "runpy.run_module('apxm_dspy', run_name='__main__')\n";
  return code;
}

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
    auto cacheDirAttr =
        module->getAttrOfType<StringAttr>(apxm::constants::attrs::DSPY_CACHE_DIR);
    auto optimizerAttr =
        module->getAttrOfType<StringAttr>(apxm::constants::attrs::DSPY_OPTIMIZER);
    auto autoAttr =
        module->getAttrOfType<StringAttr>(apxm::constants::attrs::DSPY_AUTO);
    auto metricAttr =
        module->getAttrOfType<StringAttr>(apxm::constants::attrs::DSPY_METRIC);
    auto noCacheAttr =
        module->getAttrOfType<BoolAttr>(apxm::constants::attrs::DSPY_NO_CACHE);

    if (!backendAttr) {
      module->emitError(
          "dspy-optimize: compiler prompt optimization is enabled but no "
          "backend config was provided");
      signalPassFailure();
      return;
    }

    // 3. Find python3
    auto pythonOrErr = llvm::sys::findProgramByName("python3");
    if (!pythonOrErr) {
      module->emitError("dspy-optimize: python3 not found in PATH");
      signalPassFailure();
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
    requestObj[apxm::constants::dspy_json::TRAINING_DATA_PATH] =
        trainingAttr.getValue().str();
    if (backendAttr)
      requestObj[apxm::constants::dspy_json::BACKEND_JSON] =
          backendAttr.getValue().str();
    if (cacheDirAttr)
      requestObj[apxm::constants::dspy_json::CACHE_DIR] =
          cacheDirAttr.getValue().str();
    if (optimizerAttr)
      requestObj[apxm::constants::dspy_json::OPTIMIZER] =
          optimizerAttr.getValue().str();
    if (autoAttr)
      requestObj[apxm::constants::dspy_json::AUTO] = autoAttr.getValue().str();
    if (metricAttr)
      requestObj[apxm::constants::dspy_json::METRIC] =
          metricAttr.getValue().str();
    if (noCacheAttr)
      requestObj[apxm::constants::dspy_json::NO_CACHE] =
          noCacheAttr.getValue();

    llvm::json::Array templatesArr;
    for (auto &info : opsToOptimize) {
      llvm::json::Object tmplObj;
      tmplObj[apxm::constants::dspy_json::TEMPLATE_STR] = info.templateStr;
      templatesArr.push_back(std::move(tmplObj));
    }
    requestObj[apxm::constants::dspy_json::TEMPLATES] =
        std::move(templatesArr);

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

    // 7. Execute the compiler-owned DSPy adapter with file redirects. The
    // adapter can be installed in the Dekk environment; when running from a
    // source tree, the bootstrap adds `<repo>/tools` without mutating the
    // user's shell environment.
    std::optional<llvm::StringRef> redirects[] = {
        llvm::StringRef(requestPath),  // stdin
        llvm::StringRef(responsePath), // stdout
        std::nullopt                   // stderr (inherit for progress output)
    };

    std::string bootstrap = dspyBootstrapCode();
    llvm::SmallVector<llvm::StringRef> args = {python, "-c", bootstrap};
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

      module->emitError("dspy-optimize: subprocess " + reason);
      llvm::sys::fs::remove(responsePath);
      signalPassFailure();
      return;
    }

    // 9. Read response
    auto bufOrErr = llvm::MemoryBuffer::getFile(responsePath);
    llvm::sys::fs::remove(responsePath);

    if (!bufOrErr) {
      module->emitError("dspy-optimize: failed to read response file");
      signalPassFailure();
      return;
    }

    auto responseJson = llvm::json::parse((*bufOrErr)->getBuffer());
    if (!responseJson) {
      module->emitError("dspy-optimize: invalid JSON response");
      signalPassFailure();
      return;
    }

    auto *respObj = responseJson->getAsObject();
    if (!respObj) {
      module->emitError("dspy-optimize: response is not a JSON object");
      signalPassFailure();
      return;
    }

    auto status = respObj->getString(apxm::constants::dspy_json::STATUS);
    if (!status || *status != apxm::constants::dspy_json::STATUS_OK) {
      auto errorMsg = respObj->getString(apxm::constants::dspy_json::ERROR);
      std::string msg = errorMsg ? errorMsg->str() : "unknown error";
      module->emitError("dspy-optimize: " + msg);
      signalPassFailure();
      return;
    }

    // 10. Apply optimized templates
    OpBuilder builder(module.getContext());

    // Single template mode
    if (auto optimizedTmpl = respObj->getString(
            apxm::constants::dspy_json::OPTIMIZED_TEMPLATE)) {
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
    if (auto *resultsArr =
            respObj->getArray(apxm::constants::dspy_json::RESULTS)) {
      for (size_t i = 0;
           i < resultsArr->size() && i < opsToOptimize.size(); ++i) {
        auto *resultObj = (*resultsArr)[i].getAsObject();
        if (!resultObj)
          continue;

        auto optTmpl = resultObj->getString(
            apxm::constants::dspy_json::OPTIMIZED_TEMPLATE);
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
