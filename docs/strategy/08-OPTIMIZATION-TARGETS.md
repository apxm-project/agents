# Goal-Directed Optimization Targets for APXM

**Date**: March 31, 2026
**Status**: Proposed -- extends APXM's existing -O0/-O1/-O2/-O3 levels with goal-directed targets

---

## The Idea

APXM already has optimization levels (`-O0` through `-O3`) that control *how aggressively* to optimize. But they don't control *what to optimize for*. A PR review workflow and a batch data pipeline have different optimization goals:

- The PR review wants **low latency** (developer is waiting)
- The batch pipeline wants **low cost** (runs 10,000 times/day)
- A context-limited agent wants **fewer tokens** (hitting model context limits)
- An independent multi-task workflow wants **maximum parallelism** (throughput)

**Optimization targets** tell the compiler what metric to prioritize. They compose with optimization levels:

```bash
apxm compile graph.json -O2 --target latency     # Standard passes, tuned for speed
apxm compile graph.json -O3 --target cost         # Aggressive passes, tuned for savings
apxm compile graph.json -O1 --target tokens       # Basic passes, tuned for context
```

---

## Existing Infrastructure

APXM's compiler already has everything needed to support this:

| Component | Exists | Where |
|-----------|--------|-------|
| Pass pipeline builder | Yes | `apxm-compiler/src/passes/pipeline.rs` -- `build_pass_list()` |
| Per-pass options | Yes | TableGen `Option<>` declarations in `Passes.td` |
| Pass metrics | Yes | `PassMetrics` with per-pass timing + IR size deltas |
| PGO profiles | Yes | `ExecutionProfile` with per-node latency/tokens/error stats |
| Model cost metadata | Planned (Phase 1) | `~/.apxm/models.toml` with `cost_per_1k_input/output` |
| Operation latency tiers | Yes | `OperationLatency::Low/Medium/High` on every AIS op |
| Node attributes | Yes | Arbitrary key-value on every graph node |

**What's new**: A `--target` flag that selects a named optimization profile, which adjusts pass selection, pass options, and runtime configuration.

---

## The Five Optimization Targets

### Target 1: `--target tokens` (Minimize Token Usage)

**When to use**: Hitting context limits, model with small window, or just want tighter prompts.

**Compiler behavior**:

| Pass | Effect | Options Tuned |
|------|--------|--------------|
| FuseAskOps | Merge adjacent ASK chains -> fewer total prompts, less repeated context | `fusion_mode=eager`, `max_fusion_depth=10` |
| CondenseOps | Batch QMEM/UMEM -> fewer memory round-trips with preamble overhead | (default) |
| **ContextBudget** (NEW) | Analyze per-node token requirements, insert budget caps | `max_context_ratio=0.5` |
| **DeadContextElimination** (NEW) | Remove context keys not referenced by downstream nodes | (default) |
| CSE | Eliminate duplicate subexpressions that would waste tokens | (default) |
| BuildPrompt | Generate minimal placeholders, no extra instructions | `embed_instructions=false` |

**Runtime behavior**:
- ContextStack enforces `max_tokens` from context manifests
- Summaries preferred over full data chunks when budget is tight
- Context assembly uses `truncate_to_budget()` aggressively

**New compiler pass -- ContextBudget analysis**:

```cpp
// Passes.td
def ContextBudgetPass : Pass<"context-budget", "mlir::ModuleOp"> {
  let summary = "Analyze and enforce per-node token budgets";
  let description = [{
    For each LLM operation (ASK, THINK, REASON), estimates the token
    count of: system prompt + template + input values + context manifest.
    Inserts __context_budget attributes and warns when nodes exceed
    the model's context window.

    With --target tokens: sets conservative budgets (50% of window).
    Default: sets budgets at 80% of window.
  }];
  let options = [
    Option<"maxContextRatio", "max-context-ratio", "float", "0.8",
           "Maximum fraction of context window to use">,
    Option<"warnThreshold", "warn-threshold", "unsigned", "4096",
           "Warn when estimated tokens exceed this">
  ];
};
```

**Estimated token reduction**: 20-40% (from context scoping alone; fusion adds another 10-20%)

---

### Target 2: `--target parallel` (Maximize Parallelism)

**When to use**: Throughput-oriented workloads, multi-agent dispatch, independent subtasks.

**Compiler behavior**:

| Pass | Effect | Options Tuned |
|------|--------|--------------|
| **ParallelismExtraction** (NEW) | Detect independent subgraphs, remove unnecessary sequential edges | `aggressive=true` |
| CapabilityScheduling | Lower the threshold for parallel execution | `parallel_threshold=1` |
| **SpeculationInsertion** (NEW) | Insert speculative edges where memoization data suggests profitability | `min_confidence=0.6` |
| Normalize | Deduplicate reasoning contexts -> more CSE -> more independence | (default) |

**Runtime behavior**:
- DataflowScheduler uses maximum worker pool (all available cores)
- Speculation enabled with lower confidence threshold (0.6 vs default 0.8)
- ACP agents dispatched concurrently (multiple Claude Code / Codex instances)
- Graph registration with vLLM includes all parallel opportunities

**New compiler pass -- ParallelismExtraction**:

```cpp
// Passes.td
def ParallelismExtractionPass : Pass<"parallelism-extraction", "mlir::ModuleOp"> {
  let summary = "Detect and maximize parallel execution opportunities";
  let description = [{
    Analyzes the operation DAG for:
    1. Independent subgraphs that can execute concurrently
    2. Sequential edges that exist only due to authoring order (not data deps)
    3. Fan-out patterns that can be widened

    Removes unnecessary Control edges, replaces with parallel-safe markers.
    Inserts WAIT_ALL nodes where fan-in is needed.

    With --target parallel: aggressively removes sequential constraints,
    assumes operations without shared memory effects are parallel-safe.
  }];
  let options = [
    Option<"aggressive", "aggressive", "bool", "false",
           "Remove all non-data sequential edges">,
    Option<"maxFanout", "max-fanout", "unsigned", "16",
           "Maximum parallel fan-out width">
  ];
};
```

**Key insight**: ACPX flows are sequential by default. When translated to APXM graphs (or when users author graphs), there are often unnecessary sequential constraints. This pass detects and removes them:

```
BEFORE (sequential):        AFTER (parallel):
  A -> B -> C -> D            A ──> B ──┐
                                        ├──> D
                              A ──> C ──┘
                              (B and C are independent -- no shared state)
```

**Estimated speedup**: 2-10x depending on graph structure (benchmark: 10.37x for fully independent multi-agent)

---

### Target 3: `--target latency` (Minimize End-to-End Time)

**When to use**: Interactive workflows, developer-facing tools, time-sensitive operations.

**Compiler behavior**:

| Pass | Effect | Options Tuned |
|------|--------|--------------|
| FuseAskOps | Fewer LLM round-trips = less network latency | `fusion_mode=eager` |
| ParallelismExtraction | Independent work overlaps | `aggressive=true` |
| SpeculationInsertion | Start downstream early with predicted inputs | `min_confidence=0.5` |
| **PipelineInsertion** (NEW) | Mark adjacent LLM nodes for token pipelining | `pipeline_adjacent=true` |
| CapabilityScheduling | Annotate critical path for priority scheduling | (default) |

**Runtime behavior**:
- Token pipelining between adjacent LLM nodes (30-50% overlap)
- vLLM receives critical-path priority hints -> schedules those first
- Eager prefill of downstream static contexts
- ModelRouter prefers low-latency models when policy allows

**New compiler pass -- PipelineInsertion**:

```cpp
// Passes.td
def PipelineInsertionPass : Pass<"pipeline-insertion", "mlir::ModuleOp"> {
  let summary = "Insert token pipelining markers between adjacent LLM operations";
  let description = [{
    Identifies pairs of adjacent LLM operations (ASK->ASK, ASK->THINK, etc.)
    where the downstream operation can begin prefilling while the upstream
    is still generating.

    For each eligible pair:
    1. Identifies pre-materializable context (system prompt, ancestor summaries)
    2. Marks the edge with __pipeline_enabled=true
    3. Annotates downstream node with __prefillable_context

    The runtime uses these annotations to start vLLM eager prefill.
  }];
  let options = [
    Option<"pipelineAdjacent", "pipeline-adjacent", "bool", "true",
           "Pipeline all adjacent LLM pairs (vs only critical path)">,
    Option<"minPrefillTokens", "min-prefill-tokens", "unsigned", "256",
           "Minimum prefillable tokens to justify pipelining overhead">
  ];
};
```

**What gets pipelined** (patent technique 3):
```
                     Time -->

Without pipelining:  [===Node A generate===]  [===Node B prefill===][===Node B generate===]
                                               ^--- waits for A to finish

With pipelining:     [===Node A generate===]
                         [==Node B prefill (static)==]   <- starts early!
                              [==B prefill (stream)==]   <- appends A's output
                                                   [===Node B generate===]
                                                    ^--- starts sooner!
Overlap: ~~~~~~~~~~~~30-50%~~~~~~~~~~~~
```

**Estimated latency reduction**: 30-50% on sequential LLM chains, 2-10x when combined with parallelism

---

### Target 4: `--target cost` (Minimize API Costs)

**When to use**: Batch workloads, high-volume pipelines, budget-constrained teams.

**Compiler behavior**:

| Pass | Effect | Options Tuned |
|------|--------|--------------|
| **ModelDowngrade** (NEW) | Replace expensive models with cheaper alternatives where quality allows | `max_downgrade_tiers=2` |
| FuseAskOps | Fewer API calls = fewer minimum charges | `fusion_mode=eager` |
| CondenseOps | Fewer memory operations = fewer overhead calls | (default) |
| ContextBudget | Smaller context = fewer input tokens billed | `max_context_ratio=0.5` |
| CSE | Eliminate duplicate computations | (default) |
| DCE | Eliminate unreachable nodes that would waste calls | (default) |

**Runtime behavior**:
- ModelRouter defaults to `budget` policy unless node specifies otherwise
- Memoization aggressive (cache everything at temperature=0)
- Speculation disabled (wasted computation on miss = wasted money)

**New compiler pass -- ModelDowngrade**:

```cpp
// Passes.td
def ModelDowngradePass : Pass<"model-downgrade", "mlir::ModuleOp"> {
  let summary = "Replace expensive model assignments with cheaper alternatives";
  let description = [{
    Analyzes each LLM operation and determines if a cheaper model
    can handle the task:

    1. Simple ASK (short template, no output_schema) -> nano/mini model
    2. ASK with output_schema (structured output) -> mini model (needs schema support)
    3. THINK (extended reasoning) -> keep top-tier (reasoning quality matters)
    4. REASON (structured reasoning) -> keep top-tier
    5. REFLECT/VERIFY -> mini model (evaluation is simpler than generation)

    Modifies model_policy attributes. Does NOT affect nodes with
    model_policy="specific:..." (explicit model pinning).
  }];
  let options = [
    Option<"maxDowngradeTiers", "max-downgrade-tiers", "unsigned", "2",
           "Maximum tiers to downgrade (1=top->fast, 2=top->budget)">,
    Option<"preserveReasoning", "preserve-reasoning", "bool", "true",
           "Never downgrade THINK/REASON operations">
  ];
};
```

**Model tier hierarchy** (from `~/.apxm/models.toml`):

```
top    -> gpt-5.4         ($0.03/1K input)    For: THINK, REASON, complex ASK
fast   -> gpt-5.4-mini    ($0.005/1K input)   For: REFLECT, VERIFY, medium ASK
budget -> gpt-5.4-nano    ($0.001/1K input)   For: simple ASK, classification
local  -> local-llama     ($0.00/1K input)    For: anything that fits locally
```

**Estimated cost reduction**: 50-80% (most graph nodes are simple ASK operations that don't need top-tier models)

---

### Target 5: `--target balanced` (Default)

**When to use**: General purpose. This is the default when no `--target` is specified.

**Compiler behavior**: Standard `-O` level passes with moderate settings. No aggressive model downgrading, no aggressive parallelism extraction, moderate fusion.

**This is what `-O2` without `--target` already does today.**

---

## Target Composition

Targets can be combined for multi-objective optimization:

```bash
# Minimize tokens AND maximize parallelism
apxm compile graph.json -O2 --target tokens,parallel

# Minimize cost AND latency (partial conflict -- cost wins on model choice, latency wins on execution)
apxm compile graph.json -O2 --target cost,latency
```

**Conflict resolution** (when targets disagree):

| Conflict | Resolution |
|----------|-----------|
| `cost` + `latency` on model choice | Cost wins (cheaper model), latency wins (pipelining, parallelism) |
| `tokens` + `parallel` on speculation | Speculation disabled (speculative results waste tokens) |
| `parallel` + `cost` on speculation | Cost wins (disable speculation -- wasted compute on miss) |
| `latency` + `tokens` on fusion | Tokens wins (aggressive fusion reduces both) -- no conflict |

**Implementation**: Each target produces a `TargetConfig` struct. When combining, configs are merged with a priority order:

```rust
pub struct TargetConfig {
    pub passes_to_enable: Vec<String>,
    pub passes_to_disable: Vec<String>,
    pub pass_options: HashMap<String, HashMap<String, String>>,
    pub runtime_config: RuntimeTargetConfig,
}

pub struct RuntimeTargetConfig {
    pub default_model_policy: Option<String>,      // "budget", "fast", "best"
    pub speculation_enabled: Option<bool>,
    pub pipelining_enabled: Option<bool>,
    pub max_parallelism: Option<usize>,
    pub memo_cache_mode: Option<String>,           // "aggressive", "normal", "disabled"
    pub context_budget_ratio: Option<f64>,
}
```

---

## How Targets Map to Patent Techniques

| Patent Technique | `tokens` | `parallel` | `latency` | `cost` | `balanced` |
|-----------------|----------|-----------|-----------|--------|-----------|
| Spaghetti-stack context (T1) | **Primary** | - | - | Enabled | - |
| Speculative execution (T2) | - | **Primary** | **Primary** | Disabled | - |
| Token pipelining (T3) | - | - | **Primary** | - | - |
| Memoization (T4) | Enabled | Enabled | Enabled | **Primary** | Enabled |
| Stage-specific loading (T5) | **Primary** | - | Enabled | Enabled | - |

---

## Implementation: Extending `build_pass_list()`

The existing `build_pass_list()` in `pipeline.rs` takes `(level, no_cse_llm)` and returns a `Vec<String>`. We extend it:

```rust
// Current signature:
pub fn build_pass_list(level: OptimizationLevel, no_cse_llm: bool) -> Vec<String>

// New signature:
pub fn build_pass_list(
    level: OptimizationLevel,
    no_cse_llm: bool,
    targets: &[OptimizationTarget],
) -> Vec<String>

pub enum OptimizationTarget {
    Tokens,
    Parallel,
    Latency,
    Cost,
    Balanced,
}
```

**How it works**:

1. Start with the base pass list for the optimization level (existing behavior)
2. For each target, compute a `TargetConfig`
3. Merge configs (enable/disable passes, adjust options)
4. Return final pass list

```rust
pub fn build_pass_list(
    level: OptimizationLevel,
    no_cse_llm: bool,
    targets: &[OptimizationTarget],
) -> Vec<String> {
    let mut passes = base_pass_list(level, no_cse_llm);

    for target in targets {
        let config = target.to_config();

        // Add target-specific passes
        for pass in &config.passes_to_enable {
            if !passes.contains(pass) {
                // Insert at the right position in the pipeline
                insert_pass_ordered(&mut passes, pass);
            }
        }

        // Remove incompatible passes
        passes.retain(|p| !config.passes_to_disable.contains(p));
    }

    passes
}
```

---

## CLI Integration

```bash
# Compile with optimization target
apxm compile graph.json -O2 --target latency -o graph.apxmobj

# Execute with target (compile + run)
apxm execute graph.json -O2 --target cost

# Run pre-compiled artifact with runtime target hints
apxm run graph.apxmobj --runtime-target cost
# (runtime-target only affects ModelRouter defaults and speculation/pipelining,
#  not compiler passes -- those were baked in at compile time)

# Show what a target does
apxm targets show latency
TARGET: latency
  Compiler passes enabled:  fuse-ask-ops, parallelism-extraction, speculation-insertion, pipeline-insertion
  Compiler passes tuned:    fusion_mode=eager, min_confidence=0.5, pipeline_adjacent=true
  Runtime effects:          pipelining=on, speculation=on, model_policy=fast
  Patent techniques used:   T2 (speculation), T3 (pipelining)
  Estimated improvement:    30-50% latency reduction on sequential LLM chains

# List all targets
apxm targets list
TARGET      DESCRIPTION                          KEY METRIC
balanced    Good all-around (default)            -
tokens      Minimize token usage                 20-40% fewer tokens
parallel    Maximize parallelism                 2-10x throughput
latency     Minimize end-to-end time             30-50% latency reduction
cost        Minimize API costs                   50-80% cost reduction

# Analyze graph with target-specific recommendations
apxm analyze graph.json --target cost
ANALYSIS: graph.json with --target cost
  Nodes eligible for model downgrade:    4/7 (ASK nodes without output_schema)
  Estimated cost reduction:              $0.12/run -> $0.03/run (75%)
  Fusible ASK chains:                    2 (saves 2 API calls)
  Memoizable nodes:                      5/7 (temperature=0)
  Recommendation:                        Use --target cost for this graph
```

---

## Interaction with PGO (Profile-Guided Optimization)

The existing PGO system (`ExecutionProfile`) feeds data back to the compiler. Optimization targets use this data:

| Target | PGO Data Used |
|--------|--------------|
| `latency` | Per-node `avg_latency_ms` -> identify bottlenecks, insert pipelining on slow nodes |
| `cost` | Per-node `avg_tokens` -> estimate cost, target highest-cost nodes for downgrade |
| `parallel` | Per-node latency variance -> speculation confidence calibration |
| `tokens` | Per-node `avg_tokens` -> calibrate context budgets to actual usage |

```bash
# Collect execution profile
apxm run graph.apxmobj --emit-metrics profile.json

# Recompile with profile data + target
apxm compile graph.json -O2 --target latency --profile profile.json -o graph-optimized.apxmobj
```

---

## New Passes Summary

| Pass | Lines (est.) | Targets | Patent Technique |
|------|-------------|---------|-----------------|
| `context-budget` | ~200 | tokens, cost | T5 (stage-specific loading) |
| `dead-context-elimination` | ~180 | tokens, cost | T1 (spaghetti-stack) |
| `parallelism-extraction` | ~250 | parallel, latency | - |
| `speculation-insertion` | ~220 | parallel, latency | T2 (speculative execution) |
| `pipeline-insertion` | ~200 | latency | T3 (token pipelining) |
| `model-downgrade` | ~180 | cost | - |
| **Total** | **~1,230** | | |

All passes follow the established pattern: TableGen definition in `Passes.td`, C++ implementation in `Transforms/`, Rust wrapper in `passes/manager.rs`, pipeline integration in `passes/pipeline.rs`.

---

## Summary

```
                    APXM Compiler
                         │
              ┌──────────┼──────────┐
              │          │          │
         -O level    --target    --profile
         (how much)  (what for)  (from data)
              │          │          │
              v          v          v
         ┌─────────────────────────────┐
         │      build_pass_list()      │
         │                             │
         │  Base passes (from -O)      │
         │  + Target passes (enable)   │
         │  + Target options (tune)    │
         │  + PGO hints (calibrate)    │
         └─────────────┬───────────────┘
                       │
              ┌────────v────────┐
              │  PassManager    │
              │  run(module)    │
              └────────┬────────┘
                       │
              ┌────────v────────┐     ┌──────────────┐
              │  Compiled       │     │ RuntimeTarget │
              │  .apxmobj       │     │ Config        │
              │  (passes baked) │     │ (model policy,│
              └────────┬────────┘     │  speculation, │
                       │              │  pipelining)  │
                       v              v
              ┌───────────────────────────┐
              │     APXM Runtime          │
              │     DataflowScheduler     │
              │     + ModelRouter          │
              │     + ContextStack         │
              │     + MemoCache            │
              │     + SpeculativeExecutor  │
              └───────────────────────────┘
```

**The key insight**: APXM's MLIR compiler pipeline is *already* the right place to make these decisions. The passes are already configurable with options via TableGen. The runtime already has the extension points (ModelRouter, ContextStack, MemoCache). What's missing is the *intent layer* -- the `--target` flag that translates user goals into specific pass configurations. That's ~200 lines of Rust in `pipeline.rs` + `targets.rs`, plus ~1,230 lines of C++ for the 6 new passes.
