# Performance-First Optimization Strategy -- APXM

**Date**: 2026-04-08
**Goal**: Minimum latency, maximum throughput, cost is secondary
**Status**: Research & Design

> **Related documents:**
> - [Optimization overview](../../optimization/overview.md) -- the multi-target optimization framework
> - [Optimization passes](../../optimization/passes.md) -- pass pipeline details (FuseAskOps, DCE, CSE, scheduling)
> - [vLLM integration](../../integrations/vllm.md) -- prefix caching, KV pinning, speculative decoding
> - [DSPy integration](../dspy-integration.md) -- prompt optimization for conciseness
> - [Quality optimization](quality.md) -- the opposite strategy (maximize correctness)
> - [Token optimization](tokens.md) -- the cost-focused strategy (minimize tokens)
> - [Production heuristics](../production-heuristics.md) -- SGLang, Autellix, and Guidance benchmarks

---

## Executive Summary

This document defines APXM's **performance-first optimization strategy** — the configuration, compiler passes, and runtime tuning needed to achieve **minimum latency and maximum throughput** when cost constraints are relaxed. Unlike the balanced default (`--target balanced`) or cost-optimized mode (`--target cost`), this strategy prioritizes **speed above all else**.

**Key techniques**:
1. **Aggressive fusion** — merge ALL fusible chains, even large ones
2. **Maximum parallelism** — extract independent subgraphs, remove sequential constraints
3. **Speculative execution** — start downstream before upstream completes
4. **Pipeline overlapping** — overlap prefill with upstream generation
5. **Warmup prefixes** — pre-fill KV cache before fan-out
6. **DSPy for conciseness** — shorter prompts via quality-preserving optimization
7. **vLLM tuning** — priority scheduling, prefix caching, speculative decoding
8. **Runtime pre-warming** — spawn agents ahead of first COMMUNICATE

**Expected speedup**: **2-10x** on graphs with parallelism opportunities, **1.3-1.5x** on sequential chains (via pipelining).

---

## Table of Contents

1. [Optimization Philosophy](#1-optimization-philosophy)
2. [Compiler-Level Optimizations](#2-compiler-level-optimizations)
3. [DSPy Optimizations for Performance](#3-dspy-optimizations-for-performance)
4. [vLLM Optimizations for Performance](#4-vllm-optimizations-for-performance)
5. [Runtime Optimizations for Performance](#5-runtime-optimizations-for-performance)
6. [Pass Configuration: `--target latency`](#6-pass-configuration---target-latency)
7. [Expected Benefits & Speedup Analysis](#7-expected-benefits--speedup-analysis)
8. [Benchmarks & Validation](#8-benchmarks--validation)
9. [Trade-offs & When NOT to Use](#9-trade-offs--when-not-to-use)
10. [Implementation Roadmap](#10-implementation-roadmap)

---

## 1. Optimization Philosophy

### Design Principles

**Performance-first optimization is NOT about**:
- Token minimization (we'll spend tokens for speed)
- Cost reduction (we'll use expensive models if they're faster)
- Quality-at-all-costs (we'll tolerate minor quality drops for major speedups)

**Performance-first optimization IS about**:
- **Latency reduction**: End-to-end time from graph submission to final result
- **Throughput maximization**: Concurrent requests/second sustained
- **Predictable P95/P99**: Low tail latency variance under load
- **Pipeline efficiency**: Overlap compute, minimize idle time

### Measurement Criteria

Success is measured by:
1. **Wall-clock time** (end-to-end graph execution)
2. **Parallelism efficiency** (actual concurrency / max theoretical)
3. **GPU utilization** (% time spent in compute vs idle)
4. **Cache hit rate** (prefix reuse effectiveness)
5. **Pipelining overlap** (% of sequential time eliminated)

---

## 2. Compiler-Level Optimizations

### 2.1 Aggressive Fusion (`fuse-ask-ops`)

**What changes**:
- **Default behavior** (O2, balanced): Conservative fusion with 5,000-token limit
- **Performance mode** (O2, latency): Aggressive fusion up to 32,000 tokens (fill context window)

**Settings**:
```rust
// apxm-compiler/src/passes/heuristics.rs
OptimizationTarget::Latency => Self {
    max_fused_template_tokens: 32_000,  // 4x increase from balanced
    min_fusion_savings_ms: 50,          // Lower threshold (accept smaller wins)
    enable_quality_guard: false,        // Disable quality checks (speed > quality)
}
```

**Impact**:
- Merge chains of 10+ operations into single calls
- Eliminate N-1 API round-trips (500-2000ms each)
- Risk: Large prompts may degrade model performance

**Example**:
```
Before (balanced):
  ASK("draft") → ASK("review") → ASK("refine")
  3 API calls × 1.5s = 4.5s

After (latency):
  ASK("draft, then review, then refine")
  1 API call = 2.0s
  Speedup: 2.25x
```

### 2.2 Parallel Scheduling (`parallelism-extraction` pass)

**What it does**:
- Detect independent subgraphs that can execute concurrently
- Remove unnecessary sequential edges (authoring order ≠ data dependencies)
- Fan out all independent operations to maximize parallelism

**Algorithm**:
```python
def extract_parallelism(graph):
    # 1. Build dependency graph (data flow only)
    deps = compute_data_dependencies(graph)

    # 2. Find independent subgraphs
    levels = topological_sort(deps)

    # 3. Remove control-only edges
    for edge in graph.edges:
        if edge.dependency == Control and not has_side_effects(edge.from, edge.to):
            remove_edge(edge)

    # 4. Insert WAIT_ALL where fan-in needed
    for node in graph.nodes:
        if len(node.predecessors) > 1:
            insert_wait_all(node)
```

**Impact**:
- **Sequential chain** (A→B→C→D) becomes **parallel** (A→[B,C,D]) if B, C, D are independent
- Theoretical speedup: **N×** where N = independent branches
- Measured: 2-10x on realistic graphs (from [findings archive](../../archive/findings-2026-04-08.md))

**Example** (code review workflow):
```
Before:
  analyze_requirements → implement → review → format
  (sequential, 4 × 3s = 12s)

After (parallelism-extraction detects independent analyze/format):
  analyze_requirements ──┐
                         ├──→ implement → review
  format_template ───────┘
  (2 parallel branches, max(3s, 3s + 3s) = 6s)
  Speedup: 2x
```

### 2.3 Speculative Execution (`speculation-insertion` pass)

**What it does**:
- Start downstream operations **before** upstream completes
- Use cached/predicted intermediate results for speculative prefill
- Abort and re-run if prediction was wrong (accept wasted work for latency wins)

**Heuristic**:
```rust
// When to insert speculative edges
fn should_speculate(upstream: &Node, downstream: &Node, profile: &ExecutionProfile) -> bool {
    // Check profile data for upstream output stability
    let stability = profile.output_variance(upstream.id);

    // Latency target: aggressive speculation with low confidence threshold
    stability > 0.5  // 50% confidence is enough (balanced would require 0.8)
}
```

**Settings**:
```rust
OptimizationTarget::Latency => {
    speculation_enabled: true,
    min_confidence: 0.5,  // Accept speculation with 50% hit rate
    max_wasted_work: 0.5,  // Tolerate 50% wasted computation
}
```

**Impact**:
- **Chained operations** (A→B→C) can overlap: B starts before A finishes
- Theoretical speedup: **1.3-2x** on sequential chains (from literature)
- Risk: Wasted compute on mis-speculation (cost increase)

**Example** (chained reasoning):
```
Sequential execution:
  A (2s generate) ──→ B (3s generate) ──→ C (2s generate)
  Total: 7s

Speculative execution:
  A (2s generate)
      ├─ start: B prefill (speculative A output)
      └─ end: B waits for A
             B (1s wait + 3s generate) ──→ C (similar)
  Total: ~5s (28% speedup)
```

### 2.4 Pipeline Candidate Detection (`pipeline-insertion` pass)

**What it does**:
- Identify adjacent LLM operations where downstream can start prefilling while upstream generates
- Mark edges with `__pipeline_enabled` attribute
- Annotate downstream nodes with prefillable static context

**Algorithm**:
```python
def detect_pipeline_candidates(graph):
    for edge in graph.edges:
        if edge.from.op in [ASK, THINK, REASON] and edge.to.op in [ASK, THINK, REASON]:
            # Downstream can prefill system prompt + static context
            static_tokens = estimate_static_context(edge.to)

            if static_tokens > 256:  # Min threshold for worthwhile pipelining
                edge.attributes["__pipeline_enabled"] = True
                edge.to.attributes["__prefillable_context"] = static_tokens
```

**Settings**:
```rust
OptimizationTarget::Latency => {
    pipeline_adjacent: true,         // Pipeline ALL adjacent pairs (not just critical path)
    min_prefill_tokens: 128,         // Lower threshold (accept smaller prefills)
}
```

**Impact**:
- **Sequential LLM chains** get 30-50% overlap (from vLLM benchmarks)
- Prefill happens during upstream generation (free parallelism)
- Measured: 1.51x speedup on chained_llm benchmark ([benchmark results](../../benchmarks/results/2026-04-08.md))

**Timeline**:
```
Without pipelining:
  [Node A generate (3s)] → [Node B prefill (1s) + generate (3s)]
  Total: 7s

With pipelining:
  [Node A generate (3s)]
      [Node B prefill (1s, overlaps with A's last 1s)]
                         [Node B generate (3s)]
  Total: 6s (14% speedup)
```

### 2.5 Warmup Shared Prefixes (`prompt-canonicalization` + warmup hints)

**What it does**:
- For fan-out patterns (1 input → N parallel operations), reorder prompts to extract shared prefix
- Mark first operation as `warmup_candidate`
- vLLM caches prefix on first call, downstream operations hit cache

**Current implementation** (from [findings archive](../../archive/findings-2026-04-08.md)):
- **70% cache hit rate** on 4-way fan-out
- **1.51x speedup** (5.39s → 3.58s)
- **4,368 tokens saved** from redundant prefill

**Settings**:
```rust
OptimizationTarget::Latency => {
    enable_prompt_canonicalization: true,
    warmup_all_fanout: true,         // Pre-warm ALL fan-out patterns
    min_shared_tokens: 128,          // Lower threshold (warm small prefixes too)
}
```

**Impact**:
- **Fan-out speedup**: 1.5-2x depending on prefix size
- **Measured**: 1.51x on shared_prefix_fanout.apxm ([findings](../../archive/findings-2026-04-08.md))

---

## 3. DSPy Optimizations for Performance

### 3.1 Prompt Shortening via DSPy

**Traditional DSPy** (from [DSPy integration](../dspy-integration.md)):
- **Goal**: Improve quality via few-shot examples
- **Result**: 15x token increase (384 → 6,163 chars)
- **Trade-off**: Better quality, worse latency/cost

**Performance-mode DSPy**:
- **Goal**: Maintain quality while minimizing token count
- **Metric**: `quality * (1 / output_length)`
- **Optimizer**: MIPROv2 with conciseness penalty

**Custom metric function**:
```python
def latency_aware_metric(example, prediction, trace=None):
    """
    Multi-objective metric: quality AND speed.

    Quality: Semantic match to ground truth
    Speed: Shorter prompts, shorter outputs
    """
    # Quality component (0-1 score)
    quality = semantic_similarity(prediction.answer, example.answer)

    # Conciseness component (penalize verbosity)
    prompt_tokens = len(prediction.rationale.split())  # DSPy's internal prompt
    output_tokens = len(prediction.answer.split())

    # Token penalty (normalized to 0-1)
    token_penalty = min(1.0, (prompt_tokens + output_tokens) / 1000)

    # Latency penalty (if trace available)
    latency_penalty = 0
    if trace:
        latency_penalty = min(1.0, trace.latency_ms / 5000)  # 5s baseline

    # Weighted combination
    return (
        0.6 * quality            # Quality is still primary
        - 0.3 * token_penalty    # But we penalize bloated prompts
        - 0.1 * latency_penalty  # And slow execution
    )
```

**DSPy optimizer configuration**:
```python
from dspy.teleprompt import MIPROv2

optimizer = MIPROv2(
    metric=latency_aware_metric,
    num_candidates=30,
    init_temperature=1.0,
    prompt_model="gpt-4o-mini",  # Use fast model for optimization
    task_model="gpt-4o",         # Target model for execution
)

# Optimize the workflow
optimized = optimizer.compile(
    student=baseline_workflow,
    trainset=training_examples,
    requires_permission_to_run=False,
    num_trials=20,  # Lower trial count for faster optimization
)
```

**Expected outcome**:
- **Baseline**: 0-shot prompts (~50 tokens each)
- **Quality-first DSPy**: Few-shot prompts (~2,000 tokens each, +40% accuracy)
- **Latency-first DSPy**: Optimized prompts (~200 tokens each, +25% accuracy)

**Impact**:
- 4x shorter than quality-first DSPy
- Still 4x longer than baseline (accept moderate token increase for quality)
- Maintains 60-70% of quality improvement with 1/5th the latency cost

### 3.2 Chain-of-Thought Reduction

**Standard CoT** (from literature):
- Improves reasoning quality (+15-30% on complex tasks)
- Adds significant latency (2-3x token generation)

**Adaptive CoT** (performance mode):
- Use CoT **only when** quality improvement justifies latency cost
- Heuristic: `quality_gain * cost_per_improvement > latency_penalty`

**Decision logic**:
```python
def should_use_cot(operation, profile):
    # Historical data: does CoT help this operation?
    quality_gain = profile.cot_quality_delta(operation.id)

    # Latency cost of CoT
    latency_increase = profile.cot_latency_delta(operation.id)

    # Latency target: only use CoT if quality gain is substantial
    return quality_gain > 0.3 and latency_increase < 2.0  # 2x latency max
```

**Impact**:
- Skip CoT on simple operations (classification, extraction)
- Use CoT only for complex reasoning (REASON, REFLECT, VERIFY)
- **Speedup**: 1.5-2x on workflows with mixed operation complexity

### 3.3 Model Downgrading (Selectively)

**Strategy**: Use faster models for non-critical operations

**Model tier mapping**:
```
Top (slow, accurate):    gpt-5.4, claude-opus-4.6     (2-5s/call, high quality)
Fast (balanced):         gpt-5.4-mini, claude-sonnet  (0.5-2s/call, good quality)
Budget (fast, adequate): gpt-5.4-nano, claude-haiku   (0.2-0.5s/call, ok quality)
Local (fastest):         Qwen-7B via vLLM             (0.1-0.3s/call, decent quality)
```

**Downgrade heuristic** (latency mode):
```rust
fn select_model(operation: &Operation, target: OptimizationTarget) -> String {
    match target {
        OptimizationTarget::Latency => {
            match operation.op_type {
                // THINK/REASON need reasoning quality — use top-tier
                OpType::THINK | OpType::REASON => "gpt-5.4",

                // ASK with output_schema needs function support — use fast-tier
                OpType::ASK if operation.has_output_schema() => "gpt-5.4-mini",

                // Simple ASK/VERIFY/REFLECT — use budget-tier for speed
                OpType::ASK | OpType::VERIFY | OpType::REFLECT => "gpt-5.4-nano",

                // Local model for trivial operations
                OpType::CONST_STR | OpType::PRINT => "local-llama",

                _ => "gpt-5.4-mini",  // Safe default
            }
        }
        _ => operation.model.unwrap_or("gpt-5.4"),  // Other targets use default
    }
}
```

**Impact**:
- **ASK operations**: 5-10x faster (2s → 0.3s via local vLLM)
- **Quality trade-off**: -5% to -15% accuracy on simple tasks (acceptable)
- **Cost reduction**: Side benefit of using cheaper/local models

---

## 4. vLLM Optimizations for Performance

### 4.1 Aggressive Prefix Caching

**Standard mode** (balanced):
- Cache TTL: 30 seconds
- Cache eviction: LRU with 85% capacity threshold

**Performance mode**:
- **Long TTL**: 5 minutes (increase prefix survival across requests)
- **High capacity**: 95% threshold (maximize cached prefixes)
- **Pin critical prefixes**: Keep shared context in cache across entire workflow

**Configuration**:
```bash
vllm serve model \
  --enable-prefix-caching \
  --prefix-cache-ttl 300 \          # 5 minutes (vs 30s default)
  --gpu-memory-utilization 0.95 \   # Aggressive cache usage
  --max-model-len 32768             # Large context window
```

**Runtime hints** (from APXM):
```rust
// apxm-backends/src/llm/backends/vllm.rs
RequestHints {
    pin_policy: PinPolicy::prefix(300_000),  // 5-minute TTL
    priority: 0,                              // Highest priority for warmup
    reuse_group: "shared-context-fanout",
}
```

**Impact**:
- **Cache hit rate**: 70% → 90% (longer TTL prevents expiry)
- **Speedup**: 1.51x → 1.8x (from [benchmark results](../../benchmarks/results/2026-04-08.md))

### 4.2 KV Pinning for Downstream Feeding

**What it is** (from [vLLM integration](../../integrations/vllm.md)):
- Pin KV cache blocks for upstream operations
- Downstream operations reuse pinned blocks instead of re-encoding

**Settings**:
```python
# Pin ALL upstream operations (not just critical path)
pin_policy = PinPolicy(
    mode="prefix",
    ttl_ms=300_000,  # 5 minutes
    priority="high",  # Prevent eviction under memory pressure
)
```

**Memory allocation**:
- Reserve 20-30% of KV cache capacity for pins
- Monitor `memory_pressure_releases` metric (should be 0)
- Scale back if memory pressure occurs

**Impact**:
- **Eliminate redundant prefill**: 100% reuse on pinned prefixes (vs 70% with standard caching)
- **Speedup**: 1.8x → 2.1x on fan-out patterns

### 4.3 Priority = 0 for Critical Path

**Standard priority** (balanced):
- Critical path: priority 0
- Normal: priority 5
- Background: priority 10

**Performance mode**:
- **All critical-path nodes**: priority 0 (boost everything on critical path)
- **Speculative/background**: priority 20 (deprioritize non-essential work)

**Effect**:
- vLLM scheduler runs critical path requests first
- Background tasks backfill GPU idle time
- **P95 latency improvement**: 20-40% under load (from literature)

### 4.4 Speculative Decoding (If Model Supports)

**What it is**:
- Use small "draft" model to predict tokens
- Large "target" model verifies predictions
- Accept draft tokens if correct, reject and re-generate if wrong

**Models with support**:
- Llama 3.1 (draft: Llama-3.1-8B, target: Llama-3.1-70B)
- Qwen (draft: Qwen-7B, target: Qwen-72B)

**Configuration**:
```bash
vllm serve meta-llama/Llama-3.1-70B-Instruct \
  --speculative-model meta-llama/Llama-3.1-8B-Instruct \
  --num-speculative-tokens 5 \
  --use-v2-block-manager
```

**Impact**:
- **Speedup**: 2-3x on generation (from vLLM benchmarks)
- **No quality loss**: Target model validates all output
- **Requirement**: 2 models loaded (increased memory usage)

### 4.5 Continuous Batching with Short Deadlines

**Standard batching**:
- Wait for batch to fill before processing
- Deadline: 100ms

**Performance mode**:
- **Aggressive batching**: Fill batches quickly
- **Short deadline**: 20ms (start processing sooner)
- **Dynamic batch size**: Adjust based on load

**Configuration**:
```python
# vLLM server config
scheduler_config = SchedulerConfig(
    max_num_seqs=256,           # Large batch size
    max_num_batched_tokens=8192,
    batching_deadline_ms=20,    # Short deadline (vs 100ms default)
)
```

**Impact**:
- **Higher throughput**: More concurrent requests
- **Lower latency**: Less queueing time per request
- **Trade-off**: May batch sub-optimally if traffic is bursty

---

## 5. Runtime Optimizations for Performance

### 5.1 Pre-Warm Agent Pool

**Problem**: First COMMUNICATE in a graph spawns agents (5-10s overhead)

**Solution**: Spawn agents **before** graph execution starts

**Implementation**:
```rust
// apxm-runtime/src/executor/agent_pool.rs
struct AgentPool {
    agents: HashMap<String, AgentHandle>,
    warm: bool,
}

impl AgentPool {
    pub async fn prewarm(&mut self, agent_names: &[String]) {
        // Spawn all agents in parallel before first COMMUNICATE
        let handles = future::join_all(
            agent_names.iter().map(|name| self.spawn_agent(name))
        ).await;

        // Cache handles for instant reuse
        for (name, handle) in agent_names.iter().zip(handles) {
            self.agents.insert(name.clone(), handle);
        }

        self.warm = true;
    }
}
```

**Usage**:
```rust
// In RuntimeExecutor::execute()
if target == OptimizationTarget::Latency {
    // Pre-warm agent pool before graph execution
    let agent_names = artifact.required_agents();
    executor.agent_pool.prewarm(&agent_names).await?;
}
```

**Impact**:
- **First COMMUNICATE**: 8s → 0.2s (40x faster)
- **Graph startup time**: -5 to -10 seconds on multi-agent workflows

### 5.2 Token Streaming to Downstream Immediately

**Current behavior** (default):
- Wait for LLM call to complete
- Serialize full output
- Pass to downstream operations

**Performance mode**:
- **Stream tokens** as they're generated
- Downstream operations start consuming **before upstream completes**
- Requires buffering + backpressure handling

**Implementation** (conceptual):
```rust
// Stream tokens to downstream as they arrive
async fn execute_ask_streaming(node: &Node, inputs: &[Value]) -> Result<Value> {
    let stream = llm_backend.generate_stream(prompt).await?;

    // Create a broadcast channel for downstream consumers
    let (tx, rx) = tokio::sync::broadcast::channel(1024);

    // Spawn task to forward stream to channel
    tokio::spawn(async move {
        while let Some(token) = stream.next().await {
            tx.send(token).ok();
        }
    });

    // Return receiver as output (downstream consumes stream)
    Ok(Value::Stream(rx))
}
```

**Impact**:
- **Sequential chains**: Overlap generation with consumption (30-50% speedup)
- **Complexity**: Requires stream-aware operation handlers
- **Risk**: Backpressure if downstream is slower than upstream

### 5.3 Parallel Tool Dispatch

**Current behavior**:
- INV operations dispatch tools sequentially
- Wait for each tool to complete before next

**Performance mode**:
- **Parallel dispatch**: Run all independent tool calls concurrently
- **Batching**: Group tool calls into single HTTP request (if backend supports)

**Implementation**:
```rust
async fn execute_inv_parallel(tools: &[ToolCall]) -> Result<Vec<ToolResult>> {
    // Dispatch all tools in parallel
    let futures = tools.iter().map(|tool| {
        dispatch_tool(tool.name, tool.params)
    });

    // Wait for all to complete
    let results = future::try_join_all(futures).await?;

    Ok(results)
}
```

**Impact**:
- **N sequential tools**: N × 200ms → 200ms (Nx speedup)
- **Example**: Web search + file read + API call: 600ms → 200ms (3x)

### 5.4 In-Memory Cache Only (Skip SQLite L2)

**Problem**: L2 SQLite cache writes add latency (10-50ms per write)

**Solution**: Latency mode uses **L1 DashMap only** (no persistence)

**Configuration**:
```rust
OptimizationTarget::Latency => {
    memo_cache_mode: MemoCacheMode::L1Only,  // Skip SQLite writes
    l1_capacity: 10_000,                      // Large in-memory cache
}
```

**Impact**:
- **Cache write latency**: 30ms → 0ms (eliminate SQLite overhead)
- **Trade-off**: Cache doesn't survive process restarts
- **Acceptable**: For latency-critical workloads, cache warm-up cost < persistence benefit

---

## 6. Pass Configuration: `--target latency`

Here's the complete configuration for `apxm compile graph.apxm -O2 --target latency`:

### Compiler Pass Settings

| Pass | Setting | Value | Why |
|------|---------|-------|-----|
| **FuseAskOps** | max_fused_template_tokens | 32,000 | Fill context window (vs 5,000 default) |
| | min_fusion_savings_ms | 50 | Accept smaller wins (vs 250 default) |
| | enable_quality_guard | false | Speed > quality |
| **PromptCanonicalization** | always_on | true | Maximize prefix reuse |
| | warmup_all_fanout | true | Warm ALL fan-out patterns |
| | min_shared_tokens | 128 | Lower threshold |
| **DeadContextElimination** | always_on | true | Fewer tokens = faster prefill |
| **ParallelismExtraction** | aggressive | true | Remove all non-data sequential edges |
| | max_fanout | 64 | Wide parallelism (vs 16 default) |
| **SpeculationInsertion** | min_confidence | 0.5 | Low threshold (accept 50% hit rate) |
| | max_wasted_work | 0.5 | Tolerate 50% waste |
| **PipelineInsertion** | pipeline_adjacent | true | Pipeline ALL adjacent pairs |
| | min_prefill_tokens | 128 | Lower threshold (vs 256 default) |
| **CapabilityScheduling** | critical_path_boost | 100 | Strong priority for critical path |
| **TemplateSpecialization** | always_on | true | Remove runtime overhead |
| **CSE** | on | true | Eliminate duplicate calls |

### Runtime Settings

| Setting | Value | Why |
|---------|-------|-----|
| **model_policy** | "fast" | Prefer faster models (gpt-5.4-mini, claude-sonnet) |
| **speculation_enabled** | true | Overlap work speculatively |
| **pipelining_enabled** | true | Overlap prefill with generation |
| **memo_cache_mode** | "l1_only" | Skip SQLite write latency |
| **max_parallelism** | usize::MAX | Use all available workers |
| **agent_pool_prewarm** | true | Spawn agents before first use |

### vLLM Backend Settings

| Setting | Value | Why |
|---------|-------|-----|
| **prefix_cache_ttl** | 300s | Long TTL (prevent expiry) |
| **gpu_memory_utilization** | 0.95 | Maximize cache capacity |
| **priority** | 0 (critical path) | Highest scheduler priority |
| **pin_policy** | "prefix" | Pin all upstream KV blocks |
| **pin_ttl_ms** | 300,000 | 5-minute pin duration |
| **speculative_decoding** | true (if supported) | 2-3x generation speedup |
| **batching_deadline_ms** | 20 | Short deadline (low queuing) |

### DSPy Optimization Settings (Optional)

| Setting | Value | Why |
|---------|-------|-----|
| **optimizer** | MIPROv2 | Best quality + conciseness balance |
| **metric** | latency_aware_metric | Penalize verbosity |
| **num_trials** | 20 | Faster optimization (vs 40-50 default) |
| **quality_weight** | 0.6 | Prioritize quality |
| **token_weight** | 0.3 | Penalize bloat |
| **latency_weight** | 0.1 | Mild latency penalty |

---

## 7. Expected Benefits & Speedup Analysis

### Theoretical Speedup Calculation

**Baseline**: O0 (no optimization)

**Speedup components**:

| Optimization | Speedup | Graph Structure |
|--------------|---------|-----------------|
| **Fusion** (FuseAskOps) | 1.5-2x | Sequential chains (N ops → 1 op) |
| **Parallelism** (ParallelismExtraction) | 2-10x | Independent branches (N sequential → N parallel) |
| **Prefix caching** (PromptCanonicalization + vLLM) | 1.5-2x | Fan-out patterns (shared context) |
| **Pipelining** (PipelineInsertion) | 1.3-1.5x | Sequential LLM chains (overlap prefill/generate) |
| **Speculation** (SpeculationInsertion) | 1.2-1.4x | Sequential chains (overlap work) |
| **Model downgrade** | 3-10x | Simple operations (top-tier → budget-tier) |
| **Agent pre-warming** | 1.5-2x | Multi-agent workflows (eliminate spawn latency) |

**Multiplicative gains** (optimizations compose):

Example: Multi-agent code review workflow
- **Parallelism**: 4 independent reviews (4x)
- **Prefix caching**: Shared code context (1.5x)
- **Model downgrade**: 3 simple reviews use fast model (3x on those nodes)
- **Agent pre-warm**: Eliminate 8s spawn time (1.5x)

**Total speedup**: 4 × 1.5 × 1.5 = **9x**

### Real-World Benchmarks

From existing measurements ([findings archive](../../archive/findings-2026-04-08.md), [benchmarks](../../benchmarks/results/2026-04-08.md)):

| Benchmark | Graph Structure | O0 Time | O2+Latency Time | Speedup | Source |
|-----------|----------------|---------|------------------|---------|--------|
| **shared_prefix_fanout** | 4-way fan-out, shared context | 5.39s | 3.58s | **1.51x** | findings1 |
| **dead_context_stress** | 5 unused context ops | 11.99s | 6.12s | **2.00x** | findings2.1 |
| **cse_stress** | 3 duplicate prompts | 13.96s | 12.21s | **1.15x** | findings2.3 |
| **mixed_priority** | Critical + background | 4.23s | 3.79s | **1.12x** | findings5 |

**Projected** (with full latency-mode stack):

| Benchmark | Current O2 | O2+Latency | Additional Speedup | Total vs O0 |
|-----------|-----------|------------|-------------------|-------------|
| shared_prefix_fanout | 3.58s | **2.4s** | +1.5x (pipelining) | **2.25x** |
| dead_context_stress | 6.12s | **5.5s** | +1.1x (model downgrade) | **2.18x** |
| multi_agent_workflow | 25s | **8s** | +3x (parallelism + agent prewarm) | **3x** |

### Speedup Formula

```
Total Speedup = min(
    Parallelism_Speedup,
    Token_Reduction_Factor * Model_Speed_Factor * Caching_Factor
)
```

**Where**:
- `Parallelism_Speedup` = N (number of independent branches)
- `Token_Reduction_Factor` = 1 / (1 - prefix_cache_hit_rate)
- `Model_Speed_Factor` = target_model_latency / baseline_model_latency
- `Caching_Factor` = 1 / (1 - memo_cache_hit_rate)

**Example** (multi-perspective code review):
- Parallelism: 4 independent reviews → 4x
- Prefix caching: 70% hit rate → 1 / (1 - 0.7) = 3.33x
- Model: gpt-5.4-nano (0.3s) vs gpt-5.4 (2s) → 6.67x
- Memo cache: 0% (first run) → 1x

**Total**: min(4, 3.33 × 6.67 × 1) = **4x** (bottlenecked by parallelism)

---

## 8. Benchmarks & Validation

### Test Suite for Latency Mode

**Benchmark 1: Fan-Out Prefix Caching**
- Graph: `latency_fanout.apxm` (8-way fan-out, 4KB shared context)
- Expected: **2-3x speedup** vs O0
- Validates: Prefix caching, warmup hints

**Benchmark 2: Sequential LLM Chain**
- Graph: `latency_chain.apxm` (5 ASK → THINK → REASON chain)
- Expected: **1.3-1.5x speedup** vs O0
- Validates: Pipelining, speculation, fusion

**Benchmark 3: Multi-Agent Workflow**
- Graph: `latency_multi_agent.apxm` (3 agents, 2 agents parallel)
- Expected: **3-5x speedup** vs O0
- Validates: Agent pre-warming, parallelism extraction

**Benchmark 4: Mixed Operation Complexity**
- Graph: `latency_mixed.apxm` (5 simple ASK + 2 complex REASON)
- Expected: **2-4x speedup** vs O0
- Validates: Model downgrading, priority scheduling

### Measurement Methodology

**Metrics to collect**:
1. **End-to-end time** (wall clock)
2. **Per-node latency** (from trace.ndjson)
3. **Cache hit rates** (L1, prefix cache, pin hits)
4. **Parallelism efficiency** (actual concurrency / theoretical max)
5. **GPU utilization** (from vLLM metrics)

**Comparison matrix**:

| Mode | O0 (baseline) | O2 (balanced) | O2+latency |
|------|--------------|---------------|------------|
| Fusion enabled | No | Yes (5K limit) | Yes (32K limit) |
| Parallelism extraction | No | Conservative | Aggressive |
| Speculation | No | No | Yes (50% confidence) |
| Pipelining | No | Critical path only | All adjacent pairs |
| Prefix caching | No | Yes (30s TTL) | Yes (300s TTL, pinned) |
| Model policy | Default | Balanced | Fast |
| Agent prewarm | No | No | Yes |

**Expected results**:
- O0 → O2: 1.15-1.5x (current measurements)
- O2 → O2+latency: 1.5-2.5x (additional optimizations)
- **O0 → O2+latency**: **2-5x total**

---

## 9. Trade-offs & When NOT to Use

### Costs of Latency-First Optimization

**1. Increased Token Usage**
- Aggressive fusion: +20-50% tokens (larger prompts)
- Speculation: +10-30% tokens (wasted on mis-speculation)
- **Impact**: Higher API costs (partially offset by model downgrading)

**2. Potential Quality Degradation**
- Model downgrading: -5% to -15% accuracy on simple tasks
- Aggressive fusion: May degrade reasoning quality (long prompts confuse models)
- No quality guards: Risky fusions not blocked
- **Impact**: Acceptable for speed-critical workflows, not for quality-critical

**3. Higher Memory Usage**
- Long KV cache TTL: +20-30% memory consumption
- Agent pre-warming: +200-500MB per agent
- Large L1 cache: +100-500MB RAM
- **Impact**: Requires more GPU/RAM capacity

**4. Wasted Compute on Mis-Speculation**
- 50% mis-speculation rate → 50% wasted GPU time
- **Impact**: Higher GPU cost, lower effective throughput

**5. Compilation Time Increase**
- DSPy optimization: +10-20 minutes
- Parallelism analysis: +30-60 seconds
- **Impact**: Slower iteration cycles (but only at compile time)

### When to Use Latency Mode

**Use `--target latency` when**:
- User is waiting (interactive workflows, developer tools)
- SLA requires low P95/P99 latency
- Cost is not a primary constraint
- Quality degradation of 5-15% is acceptable
- Graph has parallelism opportunities (fan-out, independent branches)

**Examples**:
- Code review assistant (developer waiting for feedback)
- Interactive debugging agent (real-time suggestions)
- Multi-agent research (parallel fact-checking)

### When NOT to Use Latency Mode

**Don't use `--target latency` when**:
- Batch processing (throughput > latency)
- Quality is paramount (medical, legal, safety-critical)
- Token budget is tight (small context models)
- Cost-sensitive workloads (high-volume production)
- Sequential chains with no parallelism (pipelining is only benefit)

**Use `--target cost` instead for**:
- Data pipelines (1000s of runs/day)
- Background analytics
- Non-critical batch jobs

**Use `--target balanced` (default) for**:
- General-purpose workflows
- Unknown/mixed requirements

---

## 10. Implementation Roadmap

### Phase 1: Compiler Pass Enhancements (Weeks 1-2)

**Goal**: Implement latency-specific pass tuning

**Tasks**:
1. **Update heuristics.rs**
   - Add `OptimizationTarget::Latency` branch to `for_target()`
   - Set aggressive fusion limits (32K tokens)
   - Disable quality guards

2. **Parallelism extraction pass**
   - New pass: `parallelism-extraction` (250 lines C++)
   - Detect independent subgraphs
   - Remove unnecessary sequential edges
   - Insert WAIT_ALL nodes

3. **Speculation insertion pass**
   - New pass: `speculation-insertion` (220 lines C++)
   - Analyze PGO profiles for output stability
   - Insert speculative edges with confidence threshold
   - Mark nodes with `__speculative=true`

4. **Pipeline insertion pass**
   - New pass: `pipeline-insertion` (200 lines C++)
   - Detect adjacent LLM operations
   - Mark edges with `__pipeline_enabled`
   - Annotate prefillable context

**Deliverables**:
- 3 new MLIR passes
- Updated heuristics for latency target
- Integration tests for each pass

### Phase 2: Runtime Optimizations (Weeks 3-4)

**Goal**: Implement runtime-level speedups

**Tasks**:
1. **Agent pool pre-warming**
   - `AgentPool::prewarm()` method
   - Parallel agent spawn on executor startup
   - Cache handles for instant reuse

2. **L1-only cache mode**
   - Skip SQLite writes in latency mode
   - Large L1 DashMap (10K entries)
   - Monitor memory usage

3. **Parallel tool dispatch**
   - Concurrent INV operation execution
   - Batching for backends that support it
   - Backpressure handling

4. **Model downgrading logic**
   - Auto-select fast models for simple operations
   - Heuristic: operation type + output schema
   - Override via node attributes

**Deliverables**:
- Runtime executor enhancements (500 lines Rust)
- Configuration plumbing for latency mode
- Benchmarks for each optimization

### Phase 3: vLLM Integration (Weeks 5-6)

**Goal**: Full graph-aware vLLM scheduling

**Tasks**:
1. **Extended vLLM backend**
   - Inject priority hints into requests
   - Register graphs before execution
   - Pin KV cache blocks for upstream operations

2. **Scheduler API enhancements**
   - Long TTL support (5 minutes)
   - High-priority pin policy
   - Memory pressure monitoring

3. **Prefix caching tuning**
   - Warmup hints for first fan-out operation
   - Reuse group tracking
   - Cache hit rate metrics

**Deliverables**:
- GraphAwareVllmBackend enhancements
- Scheduler API updates
- End-to-end vLLM benchmarks

### Phase 4: DSPy Latency Optimization (Weeks 7-8)

**Goal**: Quality-preserving prompt shortening

**Tasks**:
1. **Latency-aware metric**
   - Multi-objective function (quality + tokens + latency)
   - Configurable weights
   - Integration with MIPROv2 optimizer

2. **Adaptive CoT**
   - Heuristic for when to use CoT
   - Profile-driven decision logic
   - Per-operation override

3. **Training data collection**
   - Auto-collect from execution profiles
   - Privacy filtering
   - Diversity sampling

**Deliverables**:
- DSPy integration for latency mode
- Custom metric function
- Reference examples

### Phase 5: Benchmarking & Validation (Weeks 9-10)

**Goal**: Measure real-world speedups

**Tasks**:
1. **Benchmark suite**
   - 10 representative graphs (fan-out, chain, mixed, multi-agent)
   - O0 vs O2 vs O2+latency comparison
   - Real LLM backends (not mock)

2. **Metrics collection**
   - End-to-end latency
   - Per-node breakdown
   - Cache hit rates, GPU utilization
   - Cost comparison

3. **Documentation**
   - User guide for latency mode
   - Performance tuning tips
   - Trade-off analysis

**Deliverables**:
- 10 benchmark graphs
- Performance report (this document, updated with real data)
- User guide

### Phase 6: Production Validation (Weeks 11-12)

**Goal**: Validate on real workloads

**Tasks**:
1. **Pilot deployment**
   - 3 internal use cases
   - Collect production metrics
   - Iterate on heuristics

2. **SLO attainment**
   - P95/P99 latency targets
   - Throughput under load
   - Cost vs latency trade-off analysis

3. **Documentation finalization**
   - Best practices guide
   - Troubleshooting FAQ
   - Migration guide (balanced → latency)

**Deliverables**:
- Production metrics report
- Finalized documentation
- Release notes

---

## Conclusion

APXM's **performance-first optimization strategy** combines compiler-level transformations (aggressive fusion, parallelism extraction, pipelining), runtime enhancements (agent pre-warming, L1-only cache, parallel tool dispatch), and vLLM integration (prefix caching, KV pinning, priority scheduling) to achieve **2-10x speedup** on graphs with parallelism opportunities and **1.3-1.5x speedup** on sequential chains.

**Key insight**: LLM workflows have unique optimization opportunities that traditional compilers don't address:
- **Shared context** across parallel operations (prefix caching)
- **Overlappable prefill/generation** (pipelining)
- **Speculative execution** with LLM predictions
- **Model heterogeneity** (fast models for simple tasks)

**Trade-offs**: Latency-first optimization increases token usage (+20-50%), may degrade quality (-5% to -15%), and requires more memory (+20-30%). Acceptable for interactive workflows where user latency is paramount, but not for cost-sensitive batch processing.

**Next steps**:
1. Implement Phase 1 (compiler passes) — 2 weeks
2. Benchmark on real vLLM + vendor GPU — validate 1.5-2x speedup hypothesis
3. Pilot with internal code review workflow — measure production impact
4. Document findings and update this strategy

---

## References

1. **APXM Documentation**
   - [Optimization passes](../../optimization/passes.md) -- current pass pipeline
   - [Optimization targets](../../strategy/optimization-targets.md) -- multi-objective optimization framework
   - [DSPy integration](../dspy-integration.md) -- DSPy for prompt optimization
   - [vLLM integration](../../integrations/vllm.md) -- vLLM graph-aware scheduling

2. **Benchmarks**
   - [Findings archive](../../archive/findings-2026-04-08.md) -- comprehensive optimization results
   - [Benchmark results](../../benchmarks/results/2026-04-08.md) -- vendor GPU + vLLM prefix caching (1.51x speedup)
   - [Caching guide](../../guides/caching.md) -- MemoCache effectiveness

3. **Literature**
   - vLLM paper: "Efficient Memory Management for Large Language Model Serving with PagedAttention" (2023)
   - SGLang paper: "SGLang: Efficient Execution of Structured Language Model Programs" (2024)
   - DSPy paper: "DSPy: Compiling Declarative Language Model Calls into Self-Improving Pipelines" (2023)
   - Speculative decoding: "Fast Inference from Transformers via Speculative Decoding" (2023)

4. **Code References**
   - apxm-compiler/src/passes/heuristics.rs — Optimization heuristics (production-grade, PGO-driven)
   - apxm-backends/src/llm/backends/vllm.rs — vLLM integration
   - apxm-runtime/src/executor/ — Runtime execution engine

---

**Document Metadata**:
- Date: 2026-04-08
- Authors: APXM Research Team
- Status: Research & Design
- Next Review: After Phase 1 implementation (2 weeks)
