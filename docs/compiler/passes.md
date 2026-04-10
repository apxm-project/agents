# Compiler Optimization Pipeline

How the APXM compiler transforms agent workflows from raw IR to optimized execution DAGs.

## The Pipeline at a Glance

A workflow enters as AIS MLIR and flows through three phases: **normalization** (canonical form), **optimization** (reduce LLM calls, tokens, latency), and **analysis** (annotate for the scheduler). Each phase produces IR that feeds the next.

```
                     AirModule (graph)
                              |
               +--------------+--------------+
               |     TOKEN ESTIMATION        |  (Rust, pre-MLIR)
               |                             |
               |  annotate_token_estimates   |  BPE tokenize every template,
               |                             |  set ais.est_template_tokens
               +--------------+--------------+
                              |
                        to_air() → MLIR parse
                              |
               +--------------+--------------+
               |      NORMALIZATION          |  (C++, MLIR passes)
               |                             |
               |  normalize-agent-graph      |  dedup context, lowercase attrs
               |  build-prompt               |  fill empty templates
               |  dspy-optimize              |  ML-tuned templates (optional)
               +--------------+--------------+
                              |
               +--------------+--------------+
               |       OPTIMIZATION          |
               |                             |
               |  fuse-ask-ops               |  eliminate LLM round-trips
               |  dead-context-elimination   |  prune unused context
               |  prompt-canonicalization    |  prefix reuse for KV-cache
               |  template-specialization   |  fold constants into templates
               |  schema-narrowing          |  remove unused output schemas
               |  condense-ops             |  batch memory operations
               +--------------+--------------+
                              |
               +--------------+--------------+
               |        ANALYSIS             |
               |                             |
               |  capability-scheduling      |  tier, cost, latency labels
               |  assign-priority            |  critical path + fan-out
               |  unconsumed-value-warning   |  dead code diagnostics
               |  CSE / canonicalizer /      |  standard MLIR cleanup
               |  symbol-dce                 |
               +--------------+--------------+
                              |
                       ArtifactEmitter → ExecutionDag
                              |
               +--------------+--------------+
               |   TOKEN ESTIMATE REFINEMENT |  (Rust, post-MLIR)
               |                             |
               |  refine_token_estimates     |  replace chars/4 heuristic
               |                             |  with exact BPE counts on
               |                             |  shared_prefix_est_tokens
               +--------------+--------------+
                              |
                     .apxmobj binary
```

## Optimization Levels

The pipeline is configured by `-O` level. Higher levels add more passes and O3 iterates them to a fixed point.

| Level | Strategy | Passes |
|-------|----------|--------|
| **O0** | Passthrough — no optimization | None |
| **O1** | Basic — normalization + fusion + scheduling | normalize, build-prompt, dspy-optimize, unconsumed-value-warning, scheduling, fuse-ask-ops, assign-priority, canonicalizer, CSE, symbol-dce |
| **O2** | Standard — O1 + all optimization passes | Adds: prompt-canonicalization, template-specialization, dead-context-elimination, schema-narrowing, condense-ops |
| **O3** | Aggressive — O2 iterated to fixed-point (max 10 rounds) | Same as O2, repeated until IR stops changing |

### Target Tuning

Each level can be tuned for a specific goal. This controls pass **ordering**, not which passes run.

| Target | Strategy |
|--------|----------|
| `Balanced` | Default ordering |
| `Latency` | Scheduling and fusion run first |
| `Cost` | CSE and dead-context-elimination prioritized |
| `Tokens` | Dead-context-elimination and schema-narrowing first |
| `Parallelism` | Scheduling early, sequential constraints loosened |

Source: `crates/compiler/apxm-compiler/src/passes/pipeline.rs`

---

## Token Estimation (pre-MLIR and post-MLIR)

The MLIR passes need token counts to make budgeting decisions (e.g., should fuse-ask-ops merge these two templates?). The C++ passes fall back to a `chars / 4` heuristic, but Rust bookends the pipeline with exact BPE tokenization.

### Pre-MLIR: `annotate_token_estimates`

Before the `AirModule` is lowered to MLIR text, every ASK/THINK/REASON node gets an `ais.est_template_tokens` attribute with the exact BPE token count of its static template text (placeholders like `{0}` are stripped first).

```
AirModule node:
  op = Ask
  template_str = "Analyze {0} and summarize the key findings in 3 sentences."

After annotation:
  ais.est_template_tokens = 11   ← BPE count of "Analyze  and summarize the key findings in 3 sentences."
```

The tokenizer is selected per-node based on the `model` attribute:

| Model | Tokenizer |
|-------|-----------|
| `gpt-4` (not `-o`), `gpt-3.5-turbo` | cl100k_base |
| `gpt-4o`, `o1`, `o3`, Claude, Llama, default | o200k_base |

For models without a matching BPE tokenizer (Llama, Mistral, etc.), o200k_base is used as a reasonable approximation — the counts are for scheduling hints, not billing, so ±10% accuracy is acceptable.

### Post-MLIR: `refine_token_estimates`

After the MLIR passes run and the `ArtifactEmitter` produces `ExecutionDag`s, the `PromptCanonicalization` pass will have set `shared_prefix_est_tokens` using the `chars / 4` heuristic. The Rust post-pass replaces those with exact BPE counts:

```
ExecutionDag node (after MLIR):
  shared_prefix_group = "shared_prefix_0"
  shared_prefix_est_tokens = 125    ← chars/4 heuristic from C++

After refinement:
  shared_prefix_est_tokens = 98     ← exact BPE count
```

This matters for the runtime scheduler, which uses these estimates to predict KV-cache pressure and decide when to issue warmup requests.

Source: `crates/compiler/apxm-compiler/src/token_estimate.rs`

---

## Phase 1: Normalization

### normalize-agent-graph — Canonical Form

Puts IR into a consistent shape before optimization: deduplicates context operands and lowercases string attributes.

**Context deduplication** — removes repeated values from operand lists while preserving order:

```mlir
// BEFORE: %data appears twice in context
%r = ais.ask "Analyze {0} and compare {1}" [%data, %other, %data : !ais.token] : !ais.token

// AFTER: duplicate removed, template still correct
%r = ais.ask "Analyze {0} and compare {1}" [%data, %other : !ais.token] : !ais.token
```

**String normalization** — lowercases `memory_tier` and `capability` attributes for consistent matching in later passes:

```mlir
// BEFORE
ais.qmem "find facts" stage "db" in LTM : !ais.handle

// AFTER
ais.qmem "find facts" stage "db" in ltm : !ais.handle
```

### build-prompt — Template Placeholder Generation

Fills in `"{0}"` for LLM operations that have context inputs but an empty template string. This prevents runtime errors from missing templates.

```mlir
// BEFORE: context provided but template is empty
%r = ais.ask "" [%user_input : !ais.token] : !ais.token

// AFTER: default placeholder injected
%r = ais.ask "{0}" [%user_input : !ais.token] : !ais.token
```

### dspy-optimize — Neural Prompt Optimization (optional)

Invokes Python's DSPy library as a subprocess to optimize prompt templates using ML-based algorithms (MIPROv2, BootstrapFewShot, COPRO). Only runs when the module has an `ais.dspy_training_data_path` attribute set.

```mlir
// BEFORE: hand-written template
%r = ais.ask "Review the code for bugs" [%diff : !ais.token] : !ais.token

// AFTER: DSPy-optimized template (learned from training data)
%r = ais.ask "Identify critical security issues and logic errors in" [%diff : !ais.token] : !ais.token
```

The subprocess has a 300-second timeout. On failure, original templates are preserved.

---

## Phase 2: Optimization

### fuse-ask-ops — LLM Round-Trip Elimination

**The highest-ROI pass.** Detects producer-consumer chains where one LLM call feeds directly into another and fuses them into a single call, eliminating an entire LLM round-trip (500–2000ms saved per fusion).

**Direct fusion** — when `ask A` feeds into `ask B` and A has only one consumer:

```mlir
// BEFORE: two LLM round-trips
%a = ais.ask "What is the capital of France?" : !ais.token
%b = ais.ask "What landmarks are in {0}?" [%a : !ais.token] : !ais.token

// AFTER: single LLM call with concatenated template
%fused = ais.ask "What is the capital of France?\n---\nWhat landmarks are in {0}?"
           : !ais.token
           {ais.fused_from = ["What is the capital of France?",
                               "What landmarks are in {0}?"]}
```

**Merge-chain fusion** — traces through `merge` + `const_str` chains to find the upstream producer:

```mlir
// BEFORE: ask → const_str → merge → ask (3 ops, 2 LLM calls)
%a = ais.ask "Generate analysis" [%data : !ais.token] : !ais.token
%prefix = ais.const_str "Summary: "
%merged = ais.merge %prefix, %a : !ais.token -> !ais.token
%b = ais.ask "" [%merged : !ais.token] : !ais.token

// AFTER: single fused ask (merge chain erased)
%fused = ais.ask "Generate analysis\n---\nSummary: " [%data : !ais.token] : !ais.token
```

**Guards:**
- Producer must have `hasOneUse()` — won't fuse if the result is consumed elsewhere
- Fused template is checked against a token budget (`text.size() / 4`); fusion is skipped if the combined template exceeds `maxTemplateTokens`

### dead-context-elimination — Context Pruning

Scans template strings for `{N}` placeholders and removes context operands that are never referenced. Renumbers remaining placeholders to keep indices contiguous.

```mlir
// BEFORE: template only uses {0} and {2}, but all 5 contexts are wired
%r = ais.think "{0}\nAnalyze the database schema.\n(ignoring {1}-{4})"
       [%db_schema, %api_docs, %env_config, %metrics, %security : !ais.token]
       : !ais.token

// AFTER: unused contexts removed, {2} renumbered to {1}
%r = ais.think "{0}\nAnalyze the database schema."
       [%db_schema : !ais.token]
       : !ais.token
```

**Impact:** In the `dead_context_stress` benchmark, this pass reduces a 9-node graph to 3 nodes (2.00x speedup) by eliminating ~4000 tokens of unused context per LLM call.

### prompt-canonicalization — Prefix Reuse for KV-Cache

Groups LLM operations that share the same context operands and restructures their templates so the shared context appears first. This enables vLLM's automatic prefix caching — the KV-cache for the shared prefix is computed once and reused across all operations in the group.

```mlir
// BEFORE: 4 parallel reviews, each with the same context but different instructions
%sec  = ais.ask "You are reviewing auth code...\n\nFocus on SECURITY: ..."
          [%code : !ais.token] : !ais.token
%perf = ais.ask "You are reviewing auth code...\n\nFocus on PERFORMANCE: ..."
          [%code : !ais.token] : !ais.token
%rel  = ais.ask "You are reviewing auth code...\n\nFocus on RELIABILITY: ..."
          [%code : !ais.token] : !ais.token
%scal = ais.ask "You are reviewing auth code...\n\nFocus on SCALABILITY: ..."
          [%code : !ais.token] : !ais.token

// AFTER: templates reordered so shared context ({0}) is the prefix
%sec  = ais.ask "{0}\n---\nFocus on SECURITY: ..."
          [%code : !ais.token] : !ais.token
          {ais.shared_prefix_group = "shared_prefix_0",
           ais.shared_prefix_est_tokens = 500,
           ais.warmup_candidate = true}                    // first in group: warmup hint

%perf = ais.ask "{0}\n---\nFocus on PERFORMANCE: ..."
          [%code : !ais.token] : !ais.token
          {ais.shared_prefix_group = "shared_prefix_0",
           ais.shared_prefix_est_tokens = 500}

%rel  = ais.ask "{0}\n---\nFocus on RELIABILITY: ..."
          [%code : !ais.token] : !ais.token
          {ais.shared_prefix_group = "shared_prefix_0",
           ais.shared_prefix_est_tokens = 500}

%scal = ais.ask "{0}\n---\nFocus on SCALABILITY: ..."
          [%code : !ais.token] : !ais.token
          {ais.shared_prefix_group = "shared_prefix_0",
           ais.shared_prefix_est_tokens = 500}
```

The runtime uses `ais.warmup_candidate` to send the first request in a group slightly ahead, priming the KV-cache before the parallel requests arrive.

**Impact:** In the `shared_prefix_fanout` benchmark: 1.51x speedup with vLLM (70% cache hit rate).

### template-specialization — Constant Folding

When all context operands are `ais.const_str` (known at compile time), substitutes them directly into the template and clears the context list. Eliminates runtime string interpolation.

```mlir
// BEFORE: context is a compile-time constant
%lang = ais.const_str "Rust"
%r = ais.ask "Write a hello-world in {0}" [%lang : !ais.token] : !ais.token

// AFTER: constant folded into template, context cleared
%r = ais.ask "Write a hello-world in Rust" [] : !ais.token
```

Skipped if any placeholder references a non-constant value (partial specialization is not supported).

### schema-narrowing — Output Schema Optimization

Removes `output_schema` attributes from operations whose results are never consumed. Prevents the LLM from generating structured output that no one reads.

```mlir
// BEFORE: structured output schema on a reason op, but result is unused
%r = ais.reason "Analyze data" : !ais.token {output_schema = "{name, age, address}"}
// (%r has no downstream consumers)

// AFTER: schema removed (LLM can use free-form output, cheaper)
%r = ais.reason "Analyze data" : !ais.token
```

Currently limited to full removal when all results are unused. Field-level narrowing (removing unused fields from the schema) is planned but not yet implemented.

### condense-ops — Memory Operation Batching

Batches consecutive memory operations targeting the same memory space into single operations, reducing round-trips to the memory backend.

**QMEM condensation** — consecutive reads to the same tier/stage are concatenated:

```mlir
// BEFORE: two separate memory reads
%a = ais.qmem "find facts about X" stage "belief_db" in ltm : !ais.handle
%b = ais.qmem "find facts about Y" stage "belief_db" in ltm : !ais.handle

// AFTER: single batched read
%ab = ais.qmem "find facts about X\nfind facts about Y" stage "belief_db" in ltm : !ais.handle
```

**UMEM condensation** — consecutive writes to the same tier keep only the last write (idempotent overwrite):

```mlir
// BEFORE: two writes to same tier
ais.umem %old_data into ltm : !ais.token
ais.umem %new_data into ltm : !ais.token

// AFTER: only last write survives
ais.umem %new_data into ltm : !ais.token
```

Only condenses operations with no side-effectful ops between them (checked via `mlir::isPure`).

---

## Phase 3: Analysis

### capability-scheduling — Cost & Tier Classification

Walks every operation and annotates it with scheduling metadata: tier (io/compute/reasoning/memory), intent (capability/reasoning/goal), latency class, estimated cost, and parallel-safety.

```mlir
// BEFORE: bare ask op
%r = ais.ask "Summarize this" [%data : !ais.token] : !ais.token

// AFTER: annotated with scheduling metadata
%r = ais.ask "Summarize this" [%data : !ais.token] : !ais.token
       {ais.tier = "reasoning",
        ais.intent = "reasoning",
        ais.latency = "low",
        ais.estimated_cost = 20,
        ais.parallel_safe}
```

Cost formulas reflect the latency hierarchy of AIS operations:

| Op | Latency | Cost Formula |
|----|---------|-------------|
| `ask` | low | `base + context_size * weight` |
| `reason` | medium | `base * 3 + context_size * weight` |
| `think` | high | `base * 10 + context_size * weight` |
| `inv_tool` | varies | `base + weight` (by capability keyword) |
| `plan` | varies | `base + max(1, weight/2) * context_size` |

`think` ops are never marked `parallel_safe` because they dominate the critical path.

### assign-priority — Critical Path Analysis

Performs reverse-topological DAG analysis to identify the critical path and assign priorities for the runtime scheduler.

```
   %a = ask ...          depth=3 (CRITICAL, priority=90)
        |
   %b = ask [%a]         depth=2 (NORMAL, priority=30)
       / \
  %c = ask  %d = ask     depth=1, but %c has fan-out ≥ 3 (HIGH, priority=70)
     |  |  \
    ...  ... ...
```

```mlir
// AFTER: priority and downstream node IDs annotated
%a = ais.ask "..." : !ais.token
       {ais.priority = 90,
        ais.downstream_nodes = [2, 5, 8]}
```

Priority levels:

| Level | Value | Condition |
|-------|-------|-----------|
| Critical | 90 | On the longest path (depth == critical path length) |
| High | 70 | Fan-out ≥ 3 downstream consumers |
| Normal | 30 | Everything else |

### unconsumed-value-warning — Dead Code Diagnostics

Emits compiler warnings for operation results that are never consumed. Does **not** modify the IR — purely diagnostic.

```
warning: result of ask operation is not consumed;
         consider binding with '-> name' or removing if unused
  %r = ais.ask "unused query" : !ais.token
       ^
```

Skips side-effectful operations (memory writes, tool invocations, agent spawns, control flow) since their effects matter even if the return value is unused.

### Standard MLIR Passes

Three built-in MLIR passes run at the end of the pipeline:

- **CSE** (Common Subexpression Elimination) — deduplicates identical operations
- **canonicalizer** — applies algebraic simplification patterns
- **symbol-dce** — removes unreferenced symbols (dead functions)

---

## Benchmark Results (April 2026)

| Pass | Speedup | Benchmark | Mechanism |
|------|---------|-----------|-----------|
| dead-context-elimination | **2.00x** | `dead_context_stress` | 9→3 nodes, ~4000 tokens saved |
| prompt-canonicalization + vLLM | **1.51x** | `shared_prefix_fanout` | 70% KV-cache hit rate |
| CSE | **1.15x** | `cse_stress` | Deduplicated identical subexpressions |
| fuse-ask-ops | **1.10x** | `fusion_stress` | 20→10 LLM calls (pair fusion) |

## Pass Metrics

Each pass reports whether it changed the IR and how long it took. View with `--emit-diagnostics`:

```
[pass] normalize-agent-graph: 2 edits (1.2ms)
[pass] fuse-ask-ops: 5 fusions, 0 skipped-budget (3.4ms)
[pass] dead-context-elimination: 12 values removed (0.8ms)
[pass] assign-priority: 8 nodes, critical path = 4 (0.3ms)
```

Source: `crates/compiler/apxm-compiler/src/passes/metrics.rs`
