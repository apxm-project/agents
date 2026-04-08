# Evaluation

Ten-agent evaluation of APXM conducted on April 8, 2026. Each agent independently audited one dimension (tests, benchmarks, compiler, runtime, error handling, docs, etc.) via read-only code inspection. Results below.

## Scorecard

| # | Dimension | Score | Maturity | Verdict |
|---|-----------|-------|----------|---------|
| 1 | **Test Suite** | 8/10 | Production | 1360 tests, 0 failures. Handler coverage gap (9/40 tested). |
| 2 | **Benchmark Infrastructure** | 7/10 | Beta | Operational scripts + mock backend. Fixed-latency mock hides real benefits. |
| 3 | **Compiler Passes** | 6.5/10 | Beta | 6 passes effective, 5 incomplete, 1 dead weight. See [compiler-audit.md](compiler-audit.md). |
| 4 | **Runtime Scheduler** | 8/10 | Production | Token-counting dataflow + work-stealing correct. Priority scheduling shows regression. |
| 5 | **Error Handling** | 7/10 | Beta | 50+ error codes, good retry logic. TRYCATCH broken, ~60 unwraps. See [production-readiness.md](production-readiness.md). |
| 6 | **Backend & Observability** | 7.5/10 | Beta | Health monitoring good. Circuit breaker missing. Token metrics broken. |
| 7 | **AIS Operations** | 8/10 | Production | 31 ops, well-typed. Phase 1 ISA extensions wired in Rust but not C++. |
| 8 | **Documentation** | 7/10 | Beta | Theory excellent, strategy honest. Architecture inflates pass count (93 vs ~12). |
| 9 | **Python Frontend** | 6.5/10 | Beta | 42 tests pass. API examples may reference unimplemented methods. |
| 10 | **Gap Analysis** | 8/10 | Production | Honest, well-scoped. Effort estimates realistic. |

**Composite: 7.2/10** — solid foundations, beta-quality in execution paths.

## Proven Performance

These numbers are validated with real measurements (April 8, 2026):

| Optimization | Speedup | How Measured | Graph |
|-------------|---------|-------------|-------|
| DeadContextElimination | **2.00x** | Mock backend, 9→3 nodes (-66.7%) | `dead_context_stress` |
| PromptCanonicalization + vLLM | **1.51x** | Real vLLM, 70% prefix cache hit rate | `shared_prefix_fanout` |
| Multi-GPU (8x GPU, 72B) | **6.9x** | Tensor parallelism + prefix caching | `shared_prefix_fanout` |
| CSE | **1.15x** | Mock backend, 30% node reduction | `cse_stress` |
| MemoCache (warm) | **1.09x** | L1 cache, 100% hit, 0 LLM calls | `cache_test` |
| FuseAskOps | **1.02x** | Mock backend, 2-node reduction | `fusion_stress` |

**Not yet proven**: DSPy quality improvement (+20-40% accuracy claimed from literature, not measured with real LLM).

**Known regression**: Priority scheduling 0.91x on stress test (10% slower with O2). Works on realistic mixed-priority (1.12x).

## What You Can Evaluate On Today

**3 reliable axes:**

1. **Compiler metrics (Tier 1)** — `--emit-diagnostics` gives ops_delta, pass timing, DAG stats. Reliable for regression testing.
2. **Dead context elimination** — Measurable even with fixed-latency mock. Gold standard benchmark.
3. **vLLM prefix caching** — Requires real vLLM backend. 1.51x validated, 70% cache hit rate.

**What NOT to evaluate on today:**

- Wall-clock E2E with mock backend (fixed 500ms hides real optimization benefit)
- Token savings (token metrics broken — all zeros)
- L2 cache persistence (SQLite L2 never receives writes; L1 never evicts)
- Pass-specific metrics beyond ops_delta (pass metadata not wired through FFI)

## Findings by Area

### 1. Test Suite

1360 tests across Rust (1248), Python (42), examples (53), policy (4). Zero failures. 13 ignored (WIP/platform).

**Gap**: Runtime handler test coverage is 9/40 (22.5%). Most handlers (ASK, THINK, REASON, INV, SPAWN_AGENT, etc.) lack dedicated unit tests. The 490 server tests and 176 integration tests provide indirect coverage, but handler-level edge cases (malformed LLM output, timeout, schema validation failure) are untested. See [production-readiness.md](production-readiness.md) for the untested handler list.

### 2. Benchmark Infrastructure

Operational: 11 benchmark graphs, 2 scripts (`benchmark_e2e.py`, `benchmark.py`), mock backend (557 lines). Results exist.

**Methodology gap**: Fixed-latency mock (500ms/call) conflates compiler optimization with scheduling overhead. 3 of 4 E2E benchmarks show false regressions (0.94-0.96x). The 3-tier methodology (compiler metrics → mock LLM → real LLM) is correctly defined in [benchmarks/methodology/](../benchmarks/methodology/) but Tier 2 needs variable-latency simulation.

**Known bugs**: Token metrics all-zero, L2 cache empty, `--emit-diagnostics` dropped by dekk skill wrapper, cache stats CLI reads wrong path.

### 3. Compiler Passes

14 passes at O2. Deep dive in [compiler-audit.md](compiler-audit.md).

- **Effective (6)**: fuse-ask-ops, dead-context-elimination, CSE, canonicalizer, symbol-dce, infrastructure passes
- **Incomplete (5)**: prompt-canonicalization (vLLM blocked), schema-narrowing, condense-ops, template-specialization, scheduling
- **Dead weight (1)**: unconsumed-value-warning (diagnostic overhead, no optimization)

Of 13 passes profiled across 9 benchmarks, only canonicalizer (6/9, 7 ops) and CSE (2/9, 5 ops) consistently fire.

### 4. Runtime Scheduler

Token-counting dataflow scheduler is correct. Work-stealing with 3-level hierarchy (local queue, global injectors, peer workers) matches documentation. Concurrency control and backpressure implemented.

**Issue**: Priority scheduling stress test shows 10% regression (24.6s→27.3s). Same scheduler delivers 1.12x on realistic mixed-priority graph. Root cause unknown — may be mock backend artifact or scheduler overhead on synthetic patterns.

### 5. Error Handling

50+ error codes (E001-E999) with component-based ranges. Excellent retry logic (exponential backoff 500ms-60s, jitter, error classification). Good health monitoring (DashMap, Healthy/Degraded/Unhealthy). Deep dive in [production-readiness.md](production-readiness.md).

**Critical gaps**: TRYCATCH is a pass-through (doesn't catch errors), ~60 `.unwrap()` calls in production paths (LLM JSON parsing especially risky), no circuit breaker integration (health status not used in routing), node failure recovery is full-graph restart.

### 6. Backend & Observability

4 backend protocols (OpenAI, Anthropic, vLLM/OpenAI-compat, Ollama). Per-backend health tracking via DashMap. Per-request metrics (latency, tokens, retries). Aggregated metrics (p50/p99, success rates).

**Gaps**: Health status not integrated into dispatch decisions. Token metrics broken (all zeros). No circuit breaker — unhealthy backends still receive requests.

### 7. AIS Operations

31 operation types covering reasoning (ASK, THINK, REASON), control flow (BRANCH, LOOP, TRYCATCH), agents (SPAWN_AGENT, DELEGATE), memory (QMEM, UMEM, FENCE), and tools (INV). Phase 1 ISA extensions (UpdateGoal, Guard, Claim, Pause, Resume) exist in Rust but aren't wired into C++ compiler yet.

### 8. Documentation

Theory docs (PXM foundations, AAM, AIS, scheduling) are excellent. Strategy docs are honest and realistic. Architecture.md inflates pass count (claims 93, actual ~12 per level). Getting-started guides have format confusion (.apxm vs .air). Python API examples reference methods that may not exist (g.print, g.done need verification). Design docs are proposals presented as architecture.

Full evaluation: [archive/findings-2026-04-08.md](../archive/findings-2026-04-08.md).

### 9. Python Frontend

42 tests pass. DSPy bridge (23 tests), decorators (5), graph construction (9), imports (4). 53 examples validate and compile.

**Gaps**: API examples in docs may be outdated (pipe operators `>>`, `|` need verification against proxy.py). Module exports include AgentHandle, Team, ProviderSpec not mentioned in docs. hello.py was empty (restored during audit).

### 10. Gap Analysis

6 gaps (A1-A6) well-scoped with evidence from code. Effort estimates (2-5 weeks each) are realistic. Static model assignment (A1), context management (A2), memoization (A3), speculative execution (A4), token pipelining (A5), graph import (A6).

**Assessment**: Gap analysis is honest and prioritized correctly. A1 (model routing) and A3 (memoization) are partially implemented. A4-A5 are research-phase.

## Action Items

### P0 — Before Production Use

| # | Action | Area | Risk If Skipped |
|---|--------|------|-----------------|
| 1 | Fix JSON parsing unwraps in LLM handler | Error handling | Panic on malformed LLM output |
| 2 | Verify/implement TRYCATCH error recovery | Runtime | Users expect try-catch; it's a no-op |
| 3 | Add circuit breaker (check health before dispatch) | Backends | Requests keep hitting unhealthy backends |
| 4 | Fix token metrics collection | Observability | Can't measure token savings from optimizations |

### P1 — Pre-Release

| # | Action | Area | Impact |
|---|--------|------|--------|
| 5 | Add handler-level tests (target 30/40) | Testing | Cover edge cases in ASK, THINK, INV, SPAWN_AGENT |
| 6 | Investigate priority scheduling regression | Compiler | 10% regression on stress test |
| 7 | Implement variable-latency mock backend | Benchmarks | Unblock realistic Tier 2 measurement |
| 8 | Fix L2 cache persistence (L1→L2 eviction) | Runtime | Cache doesn't survive restarts |
| 9 | Wire Phase 1 ISA extensions into C++ compiler | AIS | 5 ops exist in Rust but can't compile |

### P2 — Quality

| # | Action | Area | Impact |
|---|--------|------|--------|
| 10 | Fix architecture.md pass count (93→~12) | Docs | Misleading claim |
| 11 | Move design docs to strategy/planned | Docs | Proposals shouldn't look like architecture |
| 12 | Add pass-specific metrics to `--emit-diagnostics` | Compiler | Enable per-pass effectiveness measurement |
| 13 | Add stress tests for schema-narrowing, condense-ops | Benchmarks | 3/14 passes have zero test coverage |
| 14 | Move unconsumed-value-warning to `--warn` flag | Compiler | Remove dead weight from optimization pipeline |
| 15 | Verify Python API methods in docs match proxy.py | Docs | Examples may not work |

## Cross-References

- **Benchmark methodology**: [benchmarks/methodology/](../benchmarks/methodology/) (compiler.md, vllm.md, metrics-infrastructure.md)
- **Benchmark results**: [archive/findings-2026-04-08.md](../archive/findings-2026-04-08.md)
- **Gap analysis**: [strategy/gap-analysis.md](../strategy/gap-analysis.md)
- **Test status snapshot**: [archive/test-status-2026-04-08.md](../archive/test-status-2026-04-08.md)
- **Pass profiling data**: [archive/pass-profiling-2026-04-08.md](../archive/pass-profiling-2026-04-08.md)
- **Compiler pass deep dive**: [compiler-audit.md](compiler-audit.md)
- **Production readiness deep dive**: [production-readiness.md](production-readiness.md)
