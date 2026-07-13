/**
 * @file  DspyOptimize.cpp
 * @brief Optimize prompt templates using DSPy (Stanford NLP).
 *
 * This pass invokes python3 -m apxm_dspy as a subprocess to run DSPy's
 * prompt optimizers (MIPROv2, BootstrapFewShot, COPRO) on template strings.
 * The pass requires a complete, typed optimization request. Missing evidence,
 * unavailable execution, and incomplete responses fail the pass rather than
 * publishing a partially optimized artifact.
 *
 * Placement: immediately after build-prompt (which establishes named
 * template/input_names contracts).
 */

#include "ais/Dialect/AIS/Transforms/Passes.h"

#include "ais/Common/Constants.h"
#include "PassStatsHelpers.h"
#include "ais/Dialect/AIS/IR/AISOps.h"
#include "ais/Dialect/AIS/Support/AISDebug.h"
#include "ais/Dialect/AIS/Transforms/Placeholders.h"

#include "mlir/IR/Builders.h"
#include "llvm/ADT/TypeSwitch.h"
#include "llvm/Support/FileSystem.h"
#include "llvm/Support/JSON.h"
#include "llvm/Support/MemoryBuffer.h"
#include "llvm/Support/Path.h"
#include "llvm/Support/Program.h"
#include "llvm/Support/raw_ostream.h"

#include <cctype>
#include <optional>

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

/// Reject candidates that would weaken the positional prompt-channel contract.
static std::optional<std::string>
validateRoleAwareTemplate(Operation *op, llvm::StringRef templateStr) {
  const unsigned contextSize = op->getNumOperands();
  const auto inputNames = placeholders::readInputNames(op);
  const bool hasInputRoles = placeholders::hasInputRoles(op);
  const auto inputRoles = placeholders::readInputRoles(op);

  if (contextSize != 0) {
    if (inputNames.size() != contextSize)
      return "input_names must be positional with LLM context operands";
    if (!hasInputRoles ||
        !placeholders::inputRolesAreValid(inputRoles, contextSize))
      return "input_roles must be explicit, canonical, and positional";
  } else {
    if (!inputNames.empty())
      return "input_names must be empty when an LLM operation has no context operands";
    if (hasInputRoles &&
        !placeholders::inputRolesAreValid(inputRoles, contextSize))
      return "input_roles must be positional with LLM context operands";
  }

  const auto nameToIndex = placeholders::nameToIndex(inputNames);
  for (size_t i = 0, n = templateStr.size(); i < n; ++i) {
    if (templateStr[i] != '{')
      continue;
    if (i + 1 < n && templateStr[i + 1] == '{') {
      ++i;
      continue;
    }

    const size_t start = i + 1;
    size_t end = start;
    while (end < n &&
           (std::isalnum(static_cast<unsigned char>(templateStr[end])) ||
            templateStr[end] == '_' || templateStr[end] == '.'))
      ++end;
    if (end == start || end == n || templateStr[end] != '}')
      continue;

    const llvm::StringRef name = templateStr.slice(start, end);
    const size_t dot = name.find('.');
    const llvm::StringRef root =
        dot == llvm::StringRef::npos ? name : name.take_front(dot);
    const auto input = nameToIndex.find(root);
    if (input == nameToIndex.end())
      return "placeholder '{" + name.str() +
             "}' is not a declared input_name";
    if (!placeholders::isUserRole(inputRoles[input->second]))
      return "placeholder '{" + name.str() + "}' selects " +
             placeholders::inputRoleName(inputRoles[input->second]).str() +
             " context; only user-role inputs may appear in a template";
    i = end;
  }
  return std::nullopt;
}

struct DspyOptimizePass : impl::DspyOptimizeBase<DspyOptimizePass> {
  using DspyOptimizeBase::DspyOptimizeBase;

  void runOnOperation() override {
    APXM_AIS_DEBUG_HEADER(DspyOptimize);
    ModuleOp module = getOperation();

    // Capture IR size at entry; fire writePassStats on every
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
    if (!trainingAttr || trainingAttr.getValue().trim().empty()) {
      module->emitError(
          "dspy-optimize: compiler prompt optimization requires a non-empty "
          "training data path");
      signalPassFailure();
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

    if (!backendAttr || backendAttr.getValue().trim().empty()) {
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
    bool invalidPromptContract = false;

    module.walk([&](Operation *op) {
      if (invalidPromptContract)
        return;
      llvm::TypeSwitch<Operation *>(op)
          .Case<AskOp, ThinkOp, ReasonOp>([&](auto llmOp) {
            StringRef tmpl = llmOp.getTemplateStrAttr().getValue();
            if (!tmpl.empty()) {
              if (const auto error = validateRoleAwareTemplate(op, tmpl)) {
                op->emitError("dspy-optimize: cannot optimize template: " +
                              *error);
                invalidPromptContract = true;
                return;
              }
              opsToOptimize.push_back({op, tmpl.str()});
            }
          });
    });

    if (invalidPromptContract) {
      signalPassFailure();
      return;
    }

    if (opsToOptimize.empty()) {
      module->emitError(
          "dspy-optimize: compiler prompt optimization found no non-empty "
          "LLM templates");
      signalPassFailure();
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
      module->emitError("dspy-optimize: failed to create request file: " +
                        ec.message());
      signalPassFailure();
      return;
    }
    if (auto ec = llvm::sys::fs::createTemporaryFile("dspy-resp", "json",
                                                      responsePath)) {
      llvm::sys::fs::remove(requestPath);
      module->emitError("dspy-optimize: failed to create response file: " +
                        ec.message());
      signalPassFailure();
      return;
    }

    {
      std::error_code EC;
      llvm::raw_fd_ostream reqFile(requestPath, EC);
      if (EC) {
        module->emitError("dspy-optimize: failed to write request: " +
                          EC.message());
        llvm::sys::fs::remove(requestPath);
        llvm::sys::fs::remove(responsePath);
        signalPassFailure();
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

    // 10. Validate the entire response before changing any template.
    llvm::SmallVector<std::string> optimizedTemplates;
    const auto optimizedTmpl =
        respObj->getString(apxm::constants::dspy_json::OPTIMIZED_TEMPLATE);
    const auto *resultsArr =
        respObj->getArray(apxm::constants::dspy_json::RESULTS);
    if (optimizedTmpl && resultsArr) {
      module->emitError(
          "dspy-optimize: response must use either single-template or batch "
          "mode, not both");
      signalPassFailure();
      return;
    }
    if (optimizedTmpl) {
      if (opsToOptimize.size() != 1 || optimizedTmpl->trim().empty()) {
        module->emitError(
            "dspy-optimize: single-template response must contain one "
            "non-empty optimized template for exactly one LLM operation");
        signalPassFailure();
        return;
      }
      optimizedTemplates.push_back(optimizedTmpl->str());
    } else if (resultsArr) {
      if (resultsArr->size() != opsToOptimize.size()) {
        module->emitError(
            "dspy-optimize: batch response count does not match requested "
            "LLM templates");
        signalPassFailure();
        return;
      }
      optimizedTemplates.reserve(resultsArr->size());
      for (size_t i = 0; i < resultsArr->size(); ++i) {
        auto *resultObj = (*resultsArr)[i].getAsObject();
        if (!resultObj) {
          module->emitError("dspy-optimize: batch response entry " +
                            std::to_string(i) + " is not an object");
          signalPassFailure();
          return;
        }
        auto templateValue = resultObj->getString(
            apxm::constants::dspy_json::OPTIMIZED_TEMPLATE);
        if (!templateValue || templateValue->trim().empty()) {
          module->emitError("dspy-optimize: batch response entry " +
                            std::to_string(i) +
                            " lacks a non-empty optimized template");
          signalPassFailure();
          return;
        }
        optimizedTemplates.push_back(templateValue->str());
      }
    } else {
      module->emitError(
          "dspy-optimize: response lacks an optimized template result");
      signalPassFailure();
      return;
    }

    for (size_t i = 0; i < opsToOptimize.size(); ++i) {
      if (const auto error =
              validateRoleAwareTemplate(opsToOptimize[i].op, optimizedTemplates[i])) {
        opsToOptimize[i].op->emitError(
            "dspy-optimize: optimized template violates the prompt contract: " +
            *error);
        signalPassFailure();
        return;
      }
    }

    OpBuilder builder(module.getContext());
    for (size_t i = 0; i < opsToOptimize.size(); ++i) {
      auto &info = opsToOptimize[i];
      llvm::TypeSwitch<Operation *>(info.op)
          .Case<AskOp, ThinkOp, ReasonOp>([&](auto llmOp) {
            llmOp.setTemplateStrAttr(builder.getStringAttr(optimizedTemplates[i]));
            optimized++;
          });
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
