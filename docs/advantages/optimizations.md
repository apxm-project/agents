# A-PXM Optimization Catalog

LLVM's moat is not LLVM IR — it's the 200+ optimization passes that run on LLVM IR. Any frontend that targets LLVM gets constant folding, dead code elimination, loop vectorization, and hundreds of other optimizations for free. No single-language compiler can match this accumulated optimization library.

A-PXM needs the same moat: **reusable optimizations that make every agent faster, cheaper, and more reliable.** This document catalogs both implemented and planned optimizations, organized by when they fire (compile-time vs. runtime) and what they save (tokens, latency, cost, quality).

> For implementation details, pass pipeline ordering, and MLIR code examples, see [implementation/compiler/optimization-passes.md](../implementation/compiler/optimization-passes.md).

---

## 1. Compile-Time Optimizations (Compiler Passes)

These run during `apxm compile` and transform the AIS graph before execution.

### 1.1 Operation Fusion (FuseAskOps) — IMPLEMENTED

**What**: Merge sequential ASK→ASK chains into a single LLM call.

**LLVM analogue**: Function inlining — eliminate call overhead by merging callsite into caller.

**Impact**: Eliminates one LLM API call per fusion. Each call costs 500ms–2s and $0.001–$0.06. In a 10-node graph with 3 fusible pairs, saves ~3 calls = ~3s latency + ~$0.09.

**How it works**: When ASK-A's output feeds directly into ASK-B's prompt, and no side effects (UMEM, INV, COMM) occur between them, the two prompts are concatenated into a single combined prompt. The model performs both tasks in one inference pass.

**Conditions**: Both must be ASK (not THINK/REASON — different semantics). Producer must have single consumer. No intervening side effects. Not across TRY_CATCH or FENCE boundaries.

### 1.2 Common Subexpression Elimination (CSE) — IMPLEMENTED

**What**: Identify operations with identical opcodes and identical inputs; replace duplicates with a single operation whose output is shared.

**LLVM analogue**: Direct — LLVM's CSE pass does exactly this on LLVM IR.

**Impact**: Eliminates redundant LLM calls. If two parts of a workflow ask the same question with the same context, only one call is made.

**Limitation**: Treats LLM calls as pure (same prompt → same result). Sound at temperature=0. A `--no-cse-llm` flag to disable CSE for non-zero temperature workflows is planned but not yet wired into the CLI (see compiler TODO).

### 1.3 Dead Code Elimination (DCE) — IMPLEMENTED

**What**: Remove operations whose outputs are never consumed by any downstream operation.

**LLVM analogue**: Direct — LLVM's DCE pass removes instructions with no uses.

**Impact**: Eliminates wasted LLM calls and tool invocations. Common after CSE makes previously-needed operations redundant, or when conditional branches render subgraphs unreachable.

**Note**: The codebase also includes an `UnconsumedValueWarning` analysis pass that warns about operation results that are never consumed -- the diagnostic counterpart to DCE. It is an analysis pass (emits warnings, does not transform) and fires after optimization passes.

### 1.4 Canonicalization — IMPLEMENTED

**What**: Rewrite operations into standard forms to enable downstream passes.

**LLVM analogue**: LLVM's InstCombine and canonicalization passes.

**Impact**: Indirect — enables other passes to fire. Normalizes branch conditions (`BRANCH(false, A, B)` → `BRANCH(true, B, A)`), removes trivial sync points (`WAIT_ALL([single])` → direct edge), removes trivial merges.

### 1.5 Prompt Normalization (NormalizeAgentGraph) — IMPLEMENTED

**What**: Canonicalizes the AIS graph by deduplicating reasoning contexts, normalizing string attributes, and establishing SSA ordering invariants.

**LLVM analogue**: LLVM IR canonicalization (lowercase identifiers, normalize metadata).

**Impact**: Reduces graph size and enables more CSE opportunities by making semantically-equivalent operations syntactically identical.

### 1.6 Prompt Template Construction (BuildPrompt) — IMPLEMENTED

**What**: Generates prompt templates for operations with empty `template_str`. Ensures `{0}` placeholder exists when context operands are provided.

**LLVM analogue**: No direct analogue — domain-specific.

**Impact**: Prevents empty prompts (which produce broken LLM responses), enables prompt-level optimizations downstream.

### 1.7 Scheduling Metadata (CapabilityScheduling) — IMPLEMENTED

**What**: Classifies capabilities into execution tiers (io/compute/reasoning/memory/general) and annotates operations with cost estimates and parallel-safety markers.

**LLVM analogue**: Instruction scheduling and cost modeling in LLVM's code generator.

**Impact**: Guides the runtime scheduler to overlap work and optimize execution order. The `parallel-threshold`, `base-cost`, and `context-weight` options are configurable.

---

### PLANNED: Compile-Time Optimizations

### 1.8 Template Specialization / Partial Evaluation

**What**: When parts of a prompt template are known at compile time (graph parameters with defaults, CONST_STR values), pre-render them. This is constant folding for prompts.

**LLVM analogue**: Constant Propagation + Constant Folding — evaluate constant expressions at compile time.

**Impact**: Minor direct token savings, but enables other passes (shared prefix extraction works better on pre-specialized templates where the variable parts have been resolved).

**Implementation**: Extend existing `BuildPrompt` pass to substitute compile-time-known parameters. The infrastructure exists — `ConstStr` ops and `parameters` already provide constant values.

**Difficulty**: Low. ~1 week. Extends existing pass.

### 1.9 Shared Prefix Detection (Prompt Caching)

**What**: Detect operations that share a common prompt prefix and mark them for provider-side prompt caching (Anthropic, OpenAI, and Google all support cached prompt prefixes).

**LLVM analogue**: Common prefix/suffix optimization in string operations, or shared library deduplication.

**Impact**: **HIGH**. Prompt caching reduces input token cost by up to 90% for shared prefixes. In a workflow where 5 ASK nodes share the same system prompt + project context (2000 tokens), caching saves 8000 redundant tokens × $0.003/1K = $0.024 per run. At scale (1000 runs/day), this is $24/day saved.

**Implementation**: Compiler pass identifies groups of ASK/THINK/REASON nodes with identical `system_prompt` or shared prefix. Emits `cached_prefix_id` attribute. Runtime uses provider's cache API.

**Difficulty**: Medium. Requires prefix extraction analysis and runtime support per backend.

### 1.10 Dead Context Elimination

**What**: Remove context entries from a prompt that downstream nodes never consume. If ASK-A produces a JSON object with 10 fields, but ASK-B only uses 2, the compiler can instruct the runtime to extract only those 2 fields before passing them downstream.

**LLVM analogue**: Dead store elimination — don't write values that are never read.

**Impact**: Reduces token count in downstream prompts. In large workflows with rich intermediate results, can reduce context by 30–70%.

**Implementation**: Track which output fields each downstream consumer accesses (via template analysis). Insert extraction/projection nodes for unused fields.

**Difficulty**: Hard. Requires output schema analysis and template variable tracking.

### 1.11 Output Schema Narrowing

**What**: When a REASON node produces structured JSON output, narrow the output schema to include only fields consumed by downstream nodes.

**LLVM analogue**: Scalar replacement of aggregates (SROA) — break a struct into individual fields and eliminate unused ones.

**Impact**: Fewer output tokens = lower cost + lower latency. A REASON node producing 20 fields when only 3 are consumed wastes ~17 fields worth of generation.

**Implementation**: Analyze downstream consumers, compute minimal output schema, rewrite the REASON node's schema attribute.

**Difficulty**: Medium. Output schema is already structured (JSON Schema).

### 1.12 Conditional Hoisting

**What**: Move cheap checks (VERIFY, BRANCH on cached values) before expensive operations (ASK, REASON). If a branch condition can be resolved from existing beliefs, check it before paying for an LLM call that might be wasted.

**LLVM analogue**: Loop-invariant code motion (LICM) — hoist invariant computations out of loops.

**Impact**: Avoids wasted LLM calls on dead branches. If a BRANCH eliminates 50% of paths, hoisting saves one LLM call per eliminated path.

**Implementation**: Identify branch conditions that depend only on already-available values (beliefs, constants, prior results). Reorder the graph to evaluate them first.

**Difficulty**: Medium. Requires side-effect analysis (already exists in `effects.rs`).

### 1.13 Batch Inference

**What**: Group independent ASK nodes that target the same model into a single batch API call.

**LLVM analogue**: Loop vectorization — execute multiple independent iterations as a single SIMD operation.

**Impact**: Batch APIs (available from OpenAI, Anthropic) can reduce per-call overhead and sometimes cost. 5 independent ASK nodes → 1 batch call.

**Implementation**: Identify ASK nodes with no data dependencies between them that share the same model config. Emit a batch-request node.

**Difficulty**: Medium. Requires batch API support in backends.

### 1.14 Speculative Execution

**What**: Start likely branches before the discriminant is resolved. If a BRANCH has 80% chance of going to path A, start executing path A speculatively while computing the branch condition.

**LLVM analogue**: Branch prediction + speculative execution in CPU pipelines.

**Impact**: Reduces latency on the critical path when speculation hits. In a workflow with a VERIFY→BRANCH pattern, the "pass" path can start while VERIFY runs.

**Implementation**: Attach probability annotations (from profiling or developer hints). Schedule likely-path nodes optimistically. Cancel if speculation fails.

**Difficulty**: Hard. Requires cancellation support and cost-aware speculation (don't speculate on expensive LLM calls).

---

## 2. Runtime Optimizations

These run during `apxm execute` or `apxm run` and optimize execution dynamically.

### 2.1 Response Memoization — PLANNED

**What**: Cache LLM responses for identical prompts. If the same prompt has been seen before (in this session or across sessions), return the cached result.

**LLVM analogue**: Memoization / computed goto tables for pure functions.

**Impact**: **VERY HIGH**. Eliminates repeated LLM calls entirely. In iterative workflows where an agent retries with the same prompt (e.g., re-reading a file), each cache hit saves 500ms–2s + full token cost.

**Implementation**: Hash the (prompt, model, temperature, tools) tuple. Store results in LTM (SQLite). Invalidate on context changes. Temperature=0 results are safe to cache; temperature>0 needs TTL.

**Difficulty**: Medium. The hash function and invalidation policy are the hard parts.

### 2.2 Adaptive Model Routing — PLANNED

**What**: Route operations to cheaper models when the task is simple. Use GPT-4/Claude for reasoning, GPT-3.5/local models for classification, extraction, and formatting.

**LLVM analogue**: Register allocation tiers — expensive registers for hot values, spill to stack for cold values.

**Impact**: **HIGH**. A 10-node workflow where 3 nodes need reasoning and 7 need simple extraction: routing the 7 to a local model saves 70% of API cost with minimal quality loss.

**Implementation**: Compiler annotates nodes with complexity tier (from CapabilityScheduling pass). Runtime selects model based on tier + available backends. Quality metrics feed back to adjust routing.

**Difficulty**: Medium. Model selection heuristics need tuning. Quality monitoring is essential.

### 2.3 Token Budget Enforcement — PARTIALLY IMPLEMENTED

**What**: Stop LLM generation early when token budget is exceeded. Compact context (summarize) when approaching budget limits.

**LLVM analogue**: Stack frame size limits with spill code.

**Impact**: Prevents runaway token usage. In long agent sessions, compaction keeps the agent functional instead of hitting context limits.

**Current state**: `token_budget` attribute exists on nodes. `MAX_TOOL_ITERATIONS = 10` is hardcoded. No automatic context compaction.

**Needed**: Runtime compaction (summarize intermediate results when context exceeds threshold), configurable max_tool_iterations per node.

### 2.4 Parallel Tool Dispatch — GAP (P0)

**What**: Execute independent tool calls concurrently instead of sequentially.

**LLVM analogue**: Instruction-level parallelism (ILP) — execute independent instructions in parallel on superscalar hardware.

**Impact**: When an LLM response contains 5 independent tool calls, parallel dispatch reduces latency from 5× to 1× (wall clock).

**Current state**: Tool calls execute sequentially in a `for` loop. No `FuturesOrdered`, no concurrency control.

**Needed**: Concurrent dispatch with per-tool read/write locks for safety.

### 2.5 Memory Tier Management — PARTIALLY IMPLEMENTED

**What**: Promote hot beliefs from LTM to STM, demote cold beliefs from STM to LTM. Keep working set in fast memory.

**LLVM analogue**: Cache hierarchy management — L1/L2/L3 promotion/demotion based on access patterns.

**Impact**: Reduces QMEM latency for frequently-accessed beliefs (STM: <1ms, LTM: ~5ms).

**Current state**: Three tiers exist (STM=InMemoryBackend, LTM=SQLite/Redb, Episodic=append-only). No automatic promotion/demotion.

**Needed**: Access tracking, promotion policy, background demotion.

### 2.6 Streaming + Early Termination — GAP (P0)

**What**: Stream LLM tokens to consumers as they arrive. Allow downstream nodes to signal "done" (e.g., when a VERIFY node has enough information to decide), canceling the rest of the generation.

**LLVM analogue**: Short-circuit evaluation — stop evaluating an expression when the result is already determined.

**Impact**: Reduces latency (user sees results immediately) and cost (early termination saves output tokens).

**Current state**: No `generate_stream` on `LLMBackend` trait. Event emitter fires `LlmToken` only after full response arrives.

---

## 3. Cross-Run Optimizations

These improve performance across multiple executions.

### 3.1 Profile-Guided Optimization (PGO) — PLANNED

**What**: Measure execution time, token usage, and cost per node across runs. Use this data to guide optimization decisions: fuse the most expensive pairs, cache the most repeated prompts, route the cheapest nodes to local models.

**LLVM analogue**: Direct — LLVM PGO uses execution profiles to guide inlining, loop unrolling, and branch prediction.

**Impact**: Focuses optimization effort where it matters most. A workflow where 80% of cost comes from 2 nodes benefits most from optimizing those 2 nodes.

**Implementation**: Emit metrics per node per run. Aggregate across runs. Feed profiles back to compiler passes.

**Difficulty**: Medium. Metrics collection is easy; using them to guide passes requires pass-specific integration.

### 3.2 Capability Condensation — PLANNED

**What**: Replace a sub-DAG with a single capability invocation when a provider ships that functionality. If a workflow has `QMEM → ASK → UMEM` (read context, ask LLM, save result) and a provider offers a "stateful chat" API that does all three, condense the sub-DAG into a single INV node.

**LLVM analogue**: Library call recognition — replace a sequence of instructions with a single library call when the library provides an optimized implementation (e.g., replace a loop with `memcpy`).

**Impact**: Reduces graph size, latency, and cost. One API call instead of three sequential operations.

**Implementation**: Requires a CondenseOps compiler pass (currently gap — only expand exists via DagSplicer). Need pattern matching on sub-DAGs and a condensation registry.

**Difficulty**: Hard. Sub-DAG pattern matching and provider capability discovery are complex.

### 3.3 Learned Prompt Templates — FUTURE

**What**: Track which prompt phrasings produce better results (higher VERIFY pass rates, fewer retries). Automatically adjust templates toward higher-quality variants.

**LLVM analogue**: Auto-tuning (like ATLAS for BLAS) — empirically determine the best configuration.

**Impact**: Improves quality over time without developer intervention.

**Implementation**: A/B test prompt variants, track quality metrics, update templates in LTM.

**Difficulty**: Hard. Requires quality measurement infrastructure and safe exploration strategy.

---

## 4. Cost Optimizations

### 4.1 Token Accounting — PLANNED

**What**: Track input/output token counts and cost per node, per flow, per agent. Surface cost breakdowns so developers know where money goes.

**LLVM analogue**: Compiler cost model (used by inliner, loop unroller to estimate benefit vs. cost).

**Impact**: Enables informed optimization decisions. "This REASON node costs $0.04/call and runs 20 times per session — optimize it first."

**Implementation**: Runtime already has token counts per LLM call. Need aggregation to node/flow/agent level + emission in metrics.

**Difficulty**: Easy. Infrastructure mostly exists.

### 4.2 Quality-Aware Fusion — PLANNED

**What**: Only fuse operations when quality metrics (VERIFY pass rate, user satisfaction) remain above threshold. Fusion saves tokens but can degrade quality if the combined prompt is too complex.

**LLVM analogue**: Inlining cost model — only inline functions below a size/complexity threshold.

**Impact**: Prevents fusion from degrading output quality. A quality gate that measures before and after.

**Implementation**: Run fused and unfused variants, compare quality metrics, keep fusion only if quality holds.

**Difficulty**: Medium. Quality measurement is the hard part.

---

## Summary: Optimization Landscape

| Optimization | Type | Status | Token Savings | Latency Savings | LLVM Analogue |
|-------------|------|--------|---------------|-----------------|---------------|
| **Operation Fusion** | Compiler | Implemented | Moderate | High (−1 LLM call) | Function inlining |
| **CSE** | Compiler | Implemented | High | High (−N duplicate calls) | CSE |
| **DCE** | Compiler | Implemented | Moderate | Moderate | DCE |
| **Canonicalization** | Compiler | Implemented | Indirect | Indirect | InstCombine |
| **Normalize** | Compiler | Implemented | Low | Low | IR canonicalization |
| **BuildPrompt** | Compiler | Implemented | Low | Low | — (domain-specific) |
| **Scheduling** | Compiler | Implemented | None | Moderate | Instruction scheduling |
| Template Specialization | Compiler | Planned | Low | Low | Constant prop/folding |
| Prompt Caching | Compiler | Planned | **Very High** | Low | Shared libraries |
| Dead Context Elim | Compiler | Planned | High | Moderate | Dead store elim |
| Output Schema Narrow | Compiler | Planned | Moderate | Moderate | SROA |
| Conditional Hoisting | Compiler | Planned | Moderate | High | LICM |
| Batch Inference | Compiler | Planned | Low | High | Vectorization |
| Speculative Exec | Compiler | Planned | None | High | Branch prediction |
| Response Memoization | Runtime | Planned | **Very High** | **Very High** | Memoization |
| Model Routing | Runtime | Planned | Moderate | Moderate | Register tiers |
| Token Budget | Runtime | Partial | Variable | Variable | Stack limits |
| Parallel Dispatch | Runtime | Gap (P0) | None | **Very High** | ILP |
| Memory Tiers | Runtime | Partial | None | Low | Cache hierarchy |
| Streaming + Cancel | Runtime | Gap (P0) | Moderate | High | Short-circuit eval |
| PGO | Cross-run | Planned | Variable | Variable | PGO |
| Condensation | Cross-run | Planned | High | High | Library call recognition |
| Learned Templates | Cross-run | Future | Low | Low | Auto-tuning |
| Token Accounting | Cost | Planned | Indirect | None | Cost model |
| Quality-Aware Fusion | Cost | Planned | Variable | Variable | Inlining heuristics |

### The Compounding Advantage

Each optimization is individually valuable. Together, they compound:

1. **Canonicalization** normalizes the graph → enables more **CSE** matches
2. **CSE** eliminates duplicates → creates dead code → enables **DCE**
3. **DCE** removes dead nodes → smaller graph → enables **Fusion** of remaining nodes
4. **Scheduling** metadata → guides **Parallel Dispatch** at runtime
5. **PGO** data → guides **Fusion** and **Model Routing** decisions

This is exactly LLVM's advantage: the pass library is a compounding asset. No single-agent framework can justify building 20+ optimization passes for one agent. A-PXM builds them once and every agent benefits.

---

## 5. Implementation Priority (Impact/Effort Ranked)

### Tier 1 — Quick Wins (1–3 weeks each)

1. **Parallel Tool Dispatch** — ~10 lines changed in `llm.rs`, eliminates sequential bottleneck
2. **Token Accounting** — enables all cost optimizations, low effort, infrastructure exists
3. **Template Specialization** — extend existing `BuildPrompt` pass for compile-time-known parameters
4. **Dead Context Elimination** (simplified form) — extend existing `NormalizeAgentGraph` to prune unreferenced context operands (full inter-procedural version is Tier 3)
5. **Response Memoization** — adapt the existing SHA-256 artifact cache pattern from `apxm-driver/src/cache.rs` for LLM response caching

### Tier 2 — High-Value, Medium Effort (3–4 weeks each)

6. **Shared Prefix Extraction** — biggest single-pass cost savings (50–90% input token reduction)
7. **Adaptive Model Routing** — leverage existing `RoutingStrategy` in `apxm-backends`
8. **Batch Independent Nodes** — leverage existing `ais.parallel_safe` from CapabilityScheduling
9. **PGO Feedback** — leverage existing `--emit-metrics` infrastructure

### Tier 3 — Strategic Investments (4–8 weeks each)

10. **Output Schema Narrowing** — inter-procedural analysis, leverages existing `output_schema` runtime support
11. **Conditional Hoisting** — leverages cost estimates from CapabilityScheduling
12. **Streaming + Early Termination** — requires per-backend streaming support
13. **Speculative Execution** — requires scheduler cancellation support

### Tier 4 — Research-Grade (8+ weeks)

14. **Capability Condensation** — sub-DAG pattern matching + provider capability discovery
15. **Learned Prompt Templates** — ML feedback loop + A/B testing infrastructure

---

## The Elevator Pitch

> "If you write an agent workflow in any framework and target A-PXM, you get these optimizations *for free*:
>
> **Today (implemented):**
> 1. Sequential LLM chains are **automatically fused** into single calls — saving 500–2000ms per fusion.
> 2. Independent branches are **automatically parallelized** — no async/await, no manual concurrency.
>
> **Roadmap (planned):**
> 3. Shared prompt prefixes get **automatic caching** — cutting input token costs by 50–90%.
> 4. Tool calls **execute in parallel** when the model requests multiple tools at once.
> 5. Dead context is **automatically stripped** — reducing noise and cost.
> 6. Deterministic responses are **memoized** across runs — eliminating repeat API costs entirely.
> 7. Simple tasks are **automatically routed** to cheaper models — cutting cost 10–20× for easy work.
> 8. Execution is **profiled and re-optimized** on subsequent compilations — scheduling improves over time.
>
> None of these require code changes. They are compiler and runtime optimizations that any frontend gets simply by targeting the AIS instruction set."
>
> **The abstraction is not the value. The optimizations on top of the abstraction are the value.**
