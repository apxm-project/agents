# APXM Compiler Optimization Benchmarking Methodology

## The Problem with Current Benchmarks

Current benchmarks show 1.02-1.12x speedup from O0 to O2, which is **misleading**. The issue:

- **LLM API latency dominates**: 2-10 seconds per call
- **Compiler savings are invisible**: Sub-millisecond compilation decisions (removing 1 API call, saving 500 tokens) are drowned out by network/LLM latency
- **Wrong metrics**: Wall-clock time with real LLMs measures infrastructure, not compiler quality

The speedup we're seeing is primarily from **parallelism scheduling** (O2 runs more in parallel), NOT from the optimization passes themselves.

## What APXM Optimizations Actually Save

### 1. FuseAskOps — Eliminates API Roundtrips
**What it does**: Merges sequential `ASK → ASK` chains into a single LLM call

```mlir
// Before:
%a = ais.ask "Draft code"
%b = ais.ask "Review: {0}" [%a]

// After (O2):
%ab = ais.ask "Draft code\n---\nReview: {previous output}"
```

**What it saves**:
- **API calls**: N operations → M operations (M < N)
- **Latency**: Each eliminated call saves one roundtrip (500-2000ms at 100 tokens/sec × avg 150 tokens)
- **NOT tokens**: Fusion keeps the same content, just batches it

**Metric to report**: `fused_pairs` count (set by pass as module attribute `ais.fused_pairs`)

---

### 2. PromptCanonicalization — Enables Prefix Cache Hits
**What it does**: Reorders templates so shared context appears first

```mlir
// Before:
%a = ais.ask "As security reviewer, review: {0}" [%diff]
%b = ais.ask "As style reviewer, review: {0}" [%diff]

// After (O2):
%a = ais.ask "{0}\n---\nReview focus: security" [%diff]  // warmup_candidate
%b = ais.ask "{0}\n---\nReview focus: style" [%diff]
```

**What it saves**:
- **Prefill tokens**: vLLM caches shared prefix, avoiding re-encoding on subsequent ops
- **Token estimate**: Sum of `ais.shared_prefix_est_tokens` across all warmup candidates
- **Latency**: Prefill is ~5-10 tokens/ms, so 4000-token shared prefix saves ~400-800ms per cache hit

**Metrics to report**:
- `prompts_canonicalized` count
- `total_shared_prefix_tokens` (sum of `ais.shared_prefix_est_tokens`)
- `warmup_candidates` count (number of ops marked for warmup)

---

### 3. DeadContextElimination — Reduces Input Tokens
**What it does**: Removes context inputs that aren't referenced in the template

```mlir
// Before:
%r = ais.ask "Use {0} and {2}" [%a, %b, %c]  // %b unused

// After (O2):
%r = ais.ask "Use {0} and {1}" [%a, %c]
```

**What it saves**:
- **Input tokens**: Each removed context value saves ~512 tokens (avg estimate)
- **Cost**: At $0.01/1K tokens, 5 eliminated contexts saves ~$0.025 per execution
- **Latency**: Fewer input tokens → smaller prefill time

**Metric to report**: `dead_context_eliminated` count (total context values removed across all ops)

---

### 4. TemplateSpecialization — Eliminates Runtime Interpolation
**What it does**: Substitutes compile-time constants directly into templates

```mlir
// Before:
%const = ais.const_str "user_input_here"
%r = ais.ask "Analyze {0}" [%const]

// After (O2):
%r = ais.ask "Analyze user_input_here" []  // no context operands
```

**What it saves**:
- **Runtime ops**: Eliminates string interpolation overhead (~5-50µs per template)
- **Graph size**: Fewer edges, simpler dataflow
- **NOT significant** for LLM benchmarks (sub-millisecond savings)

**Metric to report**: `templates_specialized` count

---

### 5. SchemaNarrowing — Reduces Output Constraints
**What it does**: Removes `output_schema` when results are unused

```mlir
// Before:
%r = ais.reason "..." {output_schema = "{name, age, address}"}
// but %r is never consumed

// After (O2):
%r = ais.reason "..."  // no schema constraint
```

**What it saves**:
- **Structured output overhead**: Schema-constrained generation is slower (backtracking, validation)
- **Token overhead**: Simplified output can be shorter
- **Estimate**: ~10-20% latency reduction for structured outputs

**Metric to report**: `schemas_narrowed` count

---

### 6. CondenseOps — Batches Memory Operations
**What it does**: Merges consecutive QMEM/UMEM chains into single operations

```mlir
// Before:
%a = ais.qmem "find facts about X" in "ltm"
%b = ais.qmem "find facts about Y" in "ltm"

// After (O2):
%ab = ais.qmem "find facts about X\nfind facts about Y" in "ltm"
```

**What it saves**:
- **Memory roundtrips**: Batched queries save syscalls/network calls to memory backend
- **Latency**: Each eliminated roundtrip saves 1-50ms depending on memory tier

**Metric to report**: `condensed_ops` count (module attribute `ais.condensed_ops`)

---

### 7. CSE (Common Subexpression Elimination) — Deduplicates LLM Calls
**What it does**: Eliminates duplicate ASK/THINK/REASON ops with identical inputs

```mlir
// Before:
%a = ais.ask "Summarize {0}" [%doc]
%b = ais.ask "Summarize {0}" [%doc]  // duplicate!

// After (O2):
%a = ais.ask "Summarize {0}" [%doc]
// %b uses %a's result
```

**What it saves**:
- **API calls**: N duplicates → 1 call
- **Tokens**: Full cost of N-1 duplicate calls eliminated
- **Latency**: N × call_latency → 1 × call_latency

**Metric**: Count duplicate ops eliminated (standard MLIR CSE pass, reports via `ops_delta` in pass metrics)

---

### 8. DCE (Dead Code Elimination) — Removes Unreachable Nodes
**What it does**: Removes operations whose results are never used

**What it saves**:
- **API calls**: Entire unreachable LLM ops eliminated
- **Tokens**: Full cost of dead operations

**Metric**: `ops_delta` from `symbol-dce` pass

---

### 9. Scheduling — Improves Parallelism
**What it does**: Reorders operations to maximize parallel execution groups

**What it saves**:
- **Wall-clock latency**: More parallel ops → better utilization
- **NOT API calls or tokens**: Same work, different order

**Metrics to report**:
- `max_parallelism` (from `apxm analyze`)
- `avg_parallelism` (from runtime scheduler metrics)
- `critical_path_ms` (from `apxm analyze`)

---

## How to Measure Each Type of Saving

### Tier 1: Compiler Metrics (NO LLM EXECUTION)
These are measurable from compilation diagnostics alone:

```bash
dekk apxm compile graph.apxm -O0 --emit-diagnostics o0.json
dekk apxm compile graph.apxm -O2 --emit-diagnostics o2.json
```

**Extract from diagnostics**:
- `initial_ops` / `final_ops` → node count reduction
- `ais.fused_pairs` → API call reduction
- `ais.dead_context_eliminated` → input token reduction (× ~512 tokens each)
- `ais.prompts_canonicalized` → prefix cache opportunities
- `ais.shared_prefix_est_tokens` → estimated token savings from prefix reuse
- `ais.templates_specialized` → context values eliminated
- `ais.schemas_narrowed` → structured output overhead eliminated
- `ais.condensed_ops` → memory roundtrips saved

**Graph analysis** (no execution):
```bash
dekk apxm analyze graph.apxm --json
```
- `max_parallelism` (O0 vs O2)
- `critical_path_ms` (estimated, assumes 3000ms per LLM op)
- `sequential_ms` vs `parallel_ms` → theoretical speedup

**Expected output**:
```
Optimization: FuseAskOps
  - API calls eliminated: 3 (10 → 7 ops)
  - Estimated latency saved: 4.5-6.0 seconds (3 × 1500-2000ms)

Optimization: DeadContextElimination
  - Context values removed: 12
  - Tokens saved: ~6144 (12 × 512)
  - Cost saved: $0.06 per execution (@$0.01/1K tokens)

Optimization: PromptCanonicalization
  - Shared prefix groups: 2
  - Warmup candidates: 2
  - Tokens eligible for caching: 8192
  - Cache hits (4 ops × 8192 tokens): ~32K tokens prefill saved
  - Estimated latency saved: 3.2-6.4 seconds (@5-10 tok/ms prefill)
```

---

### Tier 2: Mock LLM Benchmarks (ISOLATED MEASUREMENT)
Use a **mock LLM backend** with configurable latency to measure compiler impact:

```python
class MockLLM:
    def __init__(self, latency_ms=1500, tokens_per_ms=0.1):
        self.latency = latency_ms
        self.tokens_per_ms = tokens_per_ms

    def generate(self, prompt, max_tokens=150):
        # Fixed latency + output tokens × rate
        time.sleep(self.latency / 1000)
        time.sleep(max_tokens / self.tokens_per_ms / 1000)
        return f"mock response ({len(prompt)} input tokens)"
```

**Measure**:
- **API call count**: Instrument mock to track `generate()` calls
- **Token count**: Sum all `len(prompt)` across calls
- **Wall-clock time**: End-to-end execution time

**Expected results**:
```
MockLLM (1500ms latency, 100 tok/s output):
  O0: 10 calls, 45,000 input tokens, 12.5 seconds
  O2: 7 calls, 38,856 input tokens, 8.8 seconds
  Speedup: 1.42x (matches theoretical: 3 calls × 1.5s = 4.5s saved)
```

---

### Tier 3: Real LLM Quality Checks
Use **real LLM** to verify optimizations don't degrade output quality:

```bash
# Run both and diff outputs
dekk apxm execute graph.apxm -O0 --emit-session > o0_session_id
dekk apxm execute graph.apxm -O2 --emit-session > o2_session_id

# Compare final outputs
diff ~/.apxm/sessions/{o0_session_id,o2_session_id}/results.json
```

**Verify**:
- Final outputs should be **semantically equivalent**
- Fusion shouldn't change meaning (template rewrite must be correct)
- DCE should only remove truly dead code

**NOT for performance**: Real LLM variance (2-10s per call) will dominate compiler savings

---

## Benchmark Graphs Designed to Stress Each Optimization

### Benchmark 1: FuseAskOps — Sequential Chain
**File**: `fusion_chain.apxm`

**Graph**:
```
ASK("draft") → ASK("review {0}") → ASK("revise {0}") → ASK("finalize {0}") → ... (10 sequential ASKs)
```

**Expected optimization**:
- O0: 10 API calls
- O2: 1 API call (all fused)
- Metric: `fused_pairs = 9`

---

### Benchmark 2: CSE — Duplicate Prompts
**File**: `cse_duplicates.apxm`

**Graph**:
```
doc = CONST_STR("...")
summary1 = ASK("Summarize {0}", [doc])
summary2 = ASK("Summarize {0}", [doc])  // duplicate
summary3 = ASK("Summarize {0}", [doc])  // duplicate
final = MERGE(summary1, summary2, summary3)
```

**Expected optimization**:
- O0: 3 ASK calls
- O2: 1 ASK call (CSE detects duplicates)
- Metric: `ops_delta = -2` from CSE pass

---

### Benchmark 3: DeadContext — Unused Inputs
**File**: `dead_context.apxm`

**Graph**:
```
a, b, c, d, e = CONST_STR(...) × 5
result = ASK("Use {0} and {2}", [a, b, c, d, e])  // only uses a and c
```

**Expected optimization**:
- O0: 5 context values (2560 tokens)
- O2: 2 context values (1024 tokens)
- Metric: `dead_context_eliminated = 3` (1536 tokens saved)

---

### Benchmark 4: PromptCanonicalization — Shared Prefix Fan-out
**File**: `shared_prefix_fanout.apxm`

**Graph**:
```
large_context = CONST_STR("4000-token document")
review_security = ASK("As security reviewer, review: {0}", [large_context])
review_perf     = ASK("As performance reviewer, review: {0}", [large_context])
review_style    = ASK("As style reviewer, review: {0}", [large_context])
review_tests    = ASK("As test reviewer, review: {0}", [large_context])
merge = MERGE(all 4 reviews)
```

**Expected optimization**:
- O0: Each op has unique prompt structure (no prefix reuse)
- O2: All ops reordered to `{0}\n---\nReview focus: <aspect>`, enabling vLLM prefix cache
- Metric: `shared_prefix_est_tokens = 4000`, `warmup_candidates = 1`, 3 cache hits

**Theoretical savings**:
- Prefill: 4000 tokens × 3 cache hits × 5-10 tok/ms = 1.2-2.4 seconds

---

### Benchmark 5: Mixed Priority — Critical Path Optimization
**File**: `mixed_priority.apxm`

**Graph**:
```
# Critical path (sequential):
c1 = ASK("step 1")
c2 = ASK("step 2", [c1])
c3 = ASK("step 3", [c2])

# Speculative (parallel):
s1 = ASK("speculative A")
s2 = ASK("speculative B")
s3 = ASK("speculative C")

# Final merge:
final = MERGE(c3, s1, s2, s3)
```

**Expected optimization**:
- O0: Sequential execution (max_parallelism = 1-2)
- O2: Scheduler runs speculatives in parallel with critical path (max_parallelism = 4)
- Metric: `critical_path_ms` unchanged, but `parallel_ms` reduced

**Theoretical speedup**:
- O0: 6 ops × 1.5s = 9 seconds
- O2: 3s (critical) + max(3 × 1.5s speculative in parallel) = 6 seconds
- Speedup: 1.5x

---

### Benchmark 6: Chained LLM — Token Pipelining (vLLM Phase 4)
**File**: `chained_llm.apxm`

**Graph**:
```
response = ASK("Question: Design a URL shortener")
analysis = THINK("Analyze: {0}", [response])  // consumer waits for producer
design = REASON("Synthesize: {0}", [analysis])  // consumer waits
```

**Expected optimization** (future):
- O0: Sequential, each op waits for full completion
- O2 + token pipelining: `THINK` starts consuming `ASK` output **while ASK is still generating**
- Metric: `time_to_first_token` for each op (should overlap)

**NOT CURRENTLY IMPLEMENTED**: This requires Phase 4 vLLM integration (token streaming between ops)

---

## Recommended Benchmark Suite Structure

```
examples/python/benchmarks/
├── 01_fusion_chain.py          # FuseAskOps
├── 02_cse_duplicates.py        # CSE
├── 03_dead_context.py          # DeadContextElimination
├── 04_shared_prefix_fanout.py  # PromptCanonicalization (already exists)
├── 05_mixed_priority.py        # Scheduling (already exists)
├── 06_chained_llm.py          # Token pipelining (already exists, future vLLM)
└── run_suite.py               # Orchestrates all benchmarks
```

Each benchmark should:
1. Generate the `.apxm` graph
2. Compile with O0 and O2, emit diagnostics
3. Execute with MockLLM backend
4. Report **compiler metrics** (tier 1) + **wall-clock with mock** (tier 2)
5. Optionally run with real LLM for quality check (tier 3)

---

## Reporting Template

```
=== BENCHMARK: FuseAskOps - Sequential Chain ===

COMPILER METRICS (Tier 1):
  O0 → O2 node count: 10 → 1 operations
  Fused pairs: 9
  Eliminated API calls: 9
  Estimated latency saved: 13.5-18.0 seconds (9 × 1.5-2.0s)

MOCK LLM EXECUTION (Tier 2):
  Mock config: 1500ms latency, 100 tok/s
  O0: 10 calls, 12.5 seconds
  O2: 1 call, 1.5 seconds
  Speedup: 8.33x ✓ (matches theoretical)

REAL LLM QUALITY (Tier 3):
  O0 output: [hash of final result]
  O2 output: [hash of final result]
  Semantic equivalence: ✓ PASS

=== BENCHMARK: PromptCanonicalization - Shared Prefix ===

COMPILER METRICS (Tier 1):
  Shared prefix groups: 1
  Warmup candidates: 1
  Prompts canonicalized: 4
  Shared prefix tokens: 4000
  Cache hit opportunities: 3 (4 ops - 1 warmup)
  Estimated token savings: 12,000 (3 hits × 4000 tokens)
  Estimated prefill latency saved: 1.2-2.4 seconds (@5-10 tok/ms)

MOCK LLM EXECUTION (Tier 2):
  Mock config: 1500ms latency, 100 tok/s, prefix_cache=True
  O0: 4 calls, 4000 tokens × 4 prefill = 16K tokens, 8.0 seconds (prefill time)
  O2: 4 calls, 4000 + 0 + 0 + 0 prefill = 4K tokens, 5.6 seconds
  Speedup: 1.43x ✓ (cache savings: 2.4s)

REAL LLM QUALITY (Tier 3):
  Outputs equivalent: ✓ PASS
```

---

## Mock LLM Implementation

```python
class MockLLMBackend:
    """Mock LLM with configurable latency and token rate."""

    def __init__(self, base_latency_ms=1500, tokens_per_sec=100, prefill_tokens_per_ms=5):
        self.base_latency = base_latency_ms / 1000
        self.output_rate = tokens_per_sec
        self.prefill_rate = prefill_tokens_per_ms
        self.cache = {}  # prefix_hash → cached_kv
        self.metrics = {
            'calls': 0,
            'input_tokens': 0,
            'output_tokens': 0,
            'cache_hits': 0,
            'cache_misses': 0,
        }

    def generate(self, prompt, max_tokens=150, warmup_group=None):
        self.metrics['calls'] += 1
        prompt_tokens = len(prompt.split())  # rough estimate
        self.metrics['input_tokens'] += prompt_tokens

        # Simulate prefill time (with caching)
        prefill_time = 0
        if warmup_group:
            cache_key = hash(warmup_group)
            if cache_key in self.cache:
                # Cache hit: only prefill the unique suffix
                suffix_tokens = prompt_tokens - self.cache[cache_key]
                prefill_time = suffix_tokens / self.prefill_rate / 1000
                self.metrics['cache_hits'] += 1
            else:
                # Cache miss: prefill everything and cache
                self.cache[cache_key] = prompt_tokens
                prefill_time = prompt_tokens / self.prefill_rate / 1000
                self.metrics['cache_misses'] += 1
        else:
            # No cache group: always full prefill
            prefill_time = prompt_tokens / self.prefill_rate / 1000

        # Simulate generation time
        generation_time = max_tokens / self.output_rate

        time.sleep(prefill_time + generation_time)

        self.metrics['output_tokens'] += max_tokens
        return f"mock response ({max_tokens} tokens)"
```

---

## Integration with APXM Runtime

To use MockLLM backend in benchmarks, we need:

1. **Backend registration**:
   ```bash
   dekk apxm backend add mock --type local --protocol mock --endpoint mock://localhost
   ```

2. **Runtime support** (add to `crates/apxm-backends/src/mock.rs`):
   - Implement `LLMBackend` trait for `MockBackend`
   - Parse `ais.shared_prefix_group` attribute from operations
   - Track cache hits/misses in metrics
   - Export metrics via `emit-metrics`

3. **Benchmark runner** (`tools/benchmark.py`):
   ```python
   def run_benchmark(graph_path):
       # Tier 1: Compiler metrics
       o0_diag = compile_with_diagnostics(graph_path, opt_level="O0")
       o2_diag = compile_with_diagnostics(graph_path, opt_level="O2")
       print_compiler_metrics(o0_diag, o2_diag)

       # Tier 2: Mock execution
       o0_mock = execute_with_mock(graph_path, opt_level="O0")
       o2_mock = execute_with_mock(graph_path, opt_level="O2")
       print_mock_metrics(o0_mock, o2_mock)

       # Tier 3: Real LLM (optional)
       if args.verify_quality:
           o0_real = execute_with_real_llm(graph_path, opt_level="O0")
           o2_real = execute_with_real_llm(graph_path, opt_level="O2")
           verify_equivalence(o0_real, o2_real)
   ```

---

## Summary: The RIGHT Metrics

| Optimization Pass          | Primary Metric                  | Savings Type         | Measurement Tier |
|----------------------------|----------------------------------|----------------------|------------------|
| FuseAskOps                 | `fused_pairs`                   | API calls, latency   | 1 (compiler)     |
| PromptCanonicalization     | `shared_prefix_est_tokens`      | Prefill tokens       | 1 (compiler)     |
| DeadContextElimination     | `dead_context_eliminated`       | Input tokens, cost   | 1 (compiler)     |
| TemplateSpecialization     | `templates_specialized`         | Runtime overhead     | 1 (compiler)     |
| SchemaNarrowing            | `schemas_narrowed`              | Output overhead      | 1 (compiler)     |
| CondenseOps                | `condensed_ops`                 | Memory roundtrips    | 1 (compiler)     |
| CSE                        | `ops_delta` (from CSE pass)     | Duplicate calls      | 1 (compiler)     |
| DCE                        | `ops_delta` (from symbol-dce)   | Dead code            | 1 (compiler)     |
| Scheduling                 | `max_parallelism`, `critical_path` | Parallelism      | 1 (analyze cmd)  |

**Wall-clock speedup with real LLM** is NOT a primary metric — it's dominated by infrastructure variance.

**Compiler metrics** are the source of truth. Mock LLM execution validates that theory matches practice. Real LLM execution verifies quality.
