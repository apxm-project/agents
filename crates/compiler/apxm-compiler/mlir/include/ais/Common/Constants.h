/**
 * @file Constants.h
 * @brief Central registry for string literals and numeric constants.
 *
 * This header keeps hard-coded tokens in one place so that:
 * - Types are caught at compile time
 * - Refactoring is trivial-change a constant here and every user picks it up
 */

#ifndef APXM_COMMON_CONSTANTS_H
#define APXM_COMMON_CONSTANTS_H

#include "llvm/ADT/StringRef.h"

namespace apxm {
namespace constants {

/// Memory space identifiers
namespace memory {
constexpr llvm::StringLiteral STM = "stm";
constexpr llvm::StringLiteral LTM = "ltm";
constexpr llvm::StringLiteral EPISODIC = "episodic";
constexpr llvm::StringLiteral DEFAULT_SPACE = "stm";
}  // namespace memory

/// Session and context identifiers
namespace session {
constexpr llvm::StringLiteral DEFAULT_SID = "default";
}  // namespace session

/// JSON and data format constants
namespace data {
constexpr llvm::StringLiteral EMPTY_JSON = "{}";
constexpr llvm::StringLiteral EMPTY_ARRAY = "[]";
}  // namespace data

/// Operation name aliases - used in parser and MLIR generation
namespace operations {
// Memory operations
constexpr llvm::StringLiteral QUERY_MEMORY = "query_memory";
constexpr llvm::StringLiteral QMEM = "qmem";
constexpr llvm::StringLiteral MEM = "mem";

// Invocation operations
constexpr llvm::StringLiteral INVOKE = "invoke";
constexpr llvm::StringLiteral LLM = "llm";
constexpr llvm::StringLiteral TOOL = "tool";

// Reasoning operations
constexpr llvm::StringLiteral REASON = "reason";
constexpr llvm::StringLiteral RSN = "rsn";
constexpr llvm::StringLiteral THINK = "think";

// Planning operations
constexpr llvm::StringLiteral PLAN = "plan";
constexpr llvm::StringLiteral PLN = "pln";

// Reflection operations
constexpr llvm::StringLiteral REFLECT = "reflect";
constexpr llvm::StringLiteral RFL = "rfl";

// Verification operations
constexpr llvm::StringLiteral VERIFY = "verify";
constexpr llvm::StringLiteral VRF = "vrf";

// Execution operations
constexpr llvm::StringLiteral EXEC = "exec";
constexpr llvm::StringLiteral EX = "ex";

// Communication operations
constexpr llvm::StringLiteral TALK = "talk";
constexpr llvm::StringLiteral TLK = "tlk";

// Synchronization operations
constexpr llvm::StringLiteral WAIT = "wait";
constexpr llvm::StringLiteral MERGE = "merge";
}  // namespace operations

/// Default file and input identifiers
namespace input {
constexpr llvm::StringLiteral DEFAULT_INPUT = "<input>";
constexpr llvm::StringLiteral STDIN = "<stdin>";
}  // namespace input

/// MLIR attribute names set by AIS passes.
/// These mirror the canonical AIS graph attribute contract in apxm-ais.
namespace attrs {

constexpr llvm::StringLiteral DIALECT_ATTR_PREFIX = "ais.";
constexpr llvm::StringLiteral PASS_STATS_FIRED_SUFFIX = "_fired_count";
constexpr llvm::StringLiteral PASS_STATS_IR_SIZE_DELTA_SUFFIX = "_ir_size_delta";

// ---- Module-level pass counters ----
constexpr llvm::StringLiteral PROMPTS_BUILT = "ais.prompts_built";
constexpr llvm::StringLiteral SCHEDULING_ANNOTATIONS = "ais.scheduling_annotations";
constexpr llvm::StringLiteral FUSED_PAIRS = "ais.fused_pairs";
constexpr llvm::StringLiteral DEAD_CONTEXT_ELIMINATED = "ais.dead_context_eliminated";
constexpr llvm::StringLiteral CONDENSED_OPS = "ais.condensed_ops";
constexpr llvm::StringLiteral TEMPLATES_SPECIALIZED = "ais.templates_specialized";
constexpr llvm::StringLiteral SCHEMAS_NARROWED = "ais.schemas_narrowed";
constexpr llvm::StringLiteral PROMPTS_CANONICALIZED = "ais.prompts_canonicalized";
constexpr llvm::StringLiteral SHARED_PREFIX_ANALYZED = "ais.shared_prefix_analyzed";
constexpr llvm::StringLiteral GRAPH_NORMALIZED = "ais.graph_normalized";
constexpr llvm::StringLiteral DSPY_OPTIMIZED = "ais.dspy_optimized";

// ---- Per-op scheduling annotations ----
constexpr llvm::StringLiteral TIER = "ais.tier";
constexpr llvm::StringLiteral INTENT = "ais.intent";
constexpr llvm::StringLiteral ESTIMATED_COST = "ais.estimated_cost";
constexpr llvm::StringLiteral LATENCY = "ais.latency";
constexpr llvm::StringLiteral PARALLEL_SAFE = "ais.parallel_safe";

// ---- Per-op fusion annotations ----
constexpr llvm::StringLiteral FUSED_FROM = "ais.fused_from";

// ---- Pre-compiled token estimates (injected by Rust before MLIR) ----
constexpr llvm::StringLiteral EST_TEMPLATE_TOKENS = "ais.est_template_tokens";

// ---- Per-op prompt canonicalization annotations ----
constexpr llvm::StringLiteral SHARED_PREFIX_GROUP = "ais.shared_prefix_group";
constexpr llvm::StringLiteral SHARED_PREFIX_EST_TOKENS = "ais.shared_prefix_est_tokens";
constexpr llvm::StringLiteral WARMUP_CANDIDATE = "ais.warmup_candidate";
constexpr llvm::StringLiteral DOWNSTREAM_NODES = "ais.downstream_nodes";
constexpr llvm::StringLiteral FANOUT_COUNT = "ais.fanout_count";
constexpr llvm::StringLiteral REMAINING_PATH_LEN = "ais.remaining_path_len";
constexpr llvm::StringLiteral LATENCY_CLASS = "ais.latency_class";
constexpr llvm::StringLiteral BATCH_GROUP = "ais.batch_group";
constexpr llvm::StringLiteral STAGE_INDEX = "ais.stage_index";
constexpr llvm::StringLiteral ESTIMATED_DYNAMIC_TOKENS = "ais.estimated_dynamic_tokens";
constexpr llvm::StringLiteral SHARED_PREFIX_GROUP_PREFIX = "shared_prefix_analysis_";

// ---- Operation-level attributes (no ais. prefix) ----
// These mirror the canonical graph attrs exported through apxm-core and match
// ODS TableGen definitions.
// No "ais." prefix because these are op arguments, not pass annotations.
constexpr llvm::StringLiteral PRIORITY = "priority";
constexpr llvm::StringLiteral TEMPLATE_STR = "template_str";
constexpr llvm::StringLiteral VALUE = "value";
/// Parallel string array: human-readable name of each Data input,
/// in operand order. Templates reference inputs as `{name}`.
constexpr llvm::StringLiteral INPUT_NAMES = "input_names";

// ---- DSPy module-level config (input from Rust) ----
constexpr llvm::StringLiteral DSPY_TRAINING_DATA_PATH = "ais.dspy_training_data_path";
constexpr llvm::StringLiteral DSPY_BACKEND_JSON = "ais.dspy_backend_json";
constexpr llvm::StringLiteral DSPY_CACHE_DIR = "ais.dspy_cache_dir";
constexpr llvm::StringLiteral DSPY_OPTIMIZER = "ais.dspy_optimizer";
constexpr llvm::StringLiteral DSPY_AUTO = "ais.dspy_auto";
constexpr llvm::StringLiteral DSPY_METRIC = "ais.dspy_metric";
constexpr llvm::StringLiteral DSPY_NO_CACHE = "ais.dspy_no_cache";

}  // namespace attrs

namespace graph_metrics {
constexpr llvm::StringLiteral LATENCY_SHORT = "short";
constexpr llvm::StringLiteral LATENCY_MEDIUM = "medium";
constexpr llvm::StringLiteral LATENCY_LONG = "long";
}  // namespace graph_metrics

/// JSON contract for the Python DSPy optimizer subprocess.
namespace dspy_json {
constexpr llvm::StringLiteral TRAINING_DATA_PATH = "training_data_path";
constexpr llvm::StringLiteral BACKEND_JSON = "backend_json";
constexpr llvm::StringLiteral CACHE_DIR = "cache_dir";
constexpr llvm::StringLiteral OPTIMIZER = "optimizer";
constexpr llvm::StringLiteral AUTO = "auto";
constexpr llvm::StringLiteral METRIC = "metric";
constexpr llvm::StringLiteral NO_CACHE = "no_cache";
constexpr llvm::StringLiteral TEMPLATE_STR = "template_str";
constexpr llvm::StringLiteral TEMPLATES = "templates";
constexpr llvm::StringLiteral STATUS = "status";
constexpr llvm::StringLiteral STATUS_OK = "ok";
constexpr llvm::StringLiteral ERROR = "error";
constexpr llvm::StringLiteral RESULTS = "results";
constexpr llvm::StringLiteral OPTIMIZED_TEMPLATE = "optimized_template";
}  // namespace dspy_json

/// Priority levels for scheduling (AssignPriority pass).
namespace priority {
constexpr unsigned CRITICAL = 90;
constexpr unsigned HIGH = 70;
constexpr unsigned NORMAL = 30;
constexpr unsigned FAN_OUT_THRESHOLD = 3;
}  // namespace priority

/// Token estimation heuristic (PromptCanonicalization pass).
namespace tokens {
constexpr unsigned CHARS_PER_TOKEN = 4;
}  // namespace tokens

/// Version and metadata
namespace meta {
constexpr uint32_t BINARY_FORMAT_VERSION = 1;
constexpr uint64_t MIN_UNIQUE_ID = 1;
}  // namespace meta

}  // namespace constants
}  // namespace apxm

#endif  // APXM_COMMON_CONSTANTS_H
