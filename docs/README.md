# APXM Documentation

**APXM (Agent Programming eXecution Model)** is a compiler and dataflow runtime for agent workflows. Like LLVM provides shared infrastructure for programming languages, APXM provides shared infrastructure for AI agents — bringing compiler optimizations, runtime efficiency, and reproducible execution to LLM orchestration.

**Version**: 0.2.0 (April 8, 2026)
**Status**: Production-ready compiler + runtime with graph-aware vLLM integration

---

## Quick Start

```bash
# Install
apxm install

# Write your first workflow
cat > hello.apxm <<EOF
{
  "name": "hello",
  "nodes": [
    {"id": 1, "name": "greeting", "op": "ASK",
     "attributes": {"template_str": "Say hello to {{name}}", "budget_tokens": 50}},
    {"id": 2, "name": "name", "op": "CONST_STR",
     "attributes": {"value": "world"}}
  ],
  "edges": [{"from": 2, "to": 1, "dependency": "Data"}]
}
EOF

# Compile and execute
apxm execute hello.apxm
```

**Or use Python**:
```python
from apxm import compile

@compile()
def hello(g):
    name = g.text(value="world")
    greeting = g.ask("Say hello to {{name}}")
    g.done(greeting)
```

---

## Key Results (April 7-8, 2026 Benchmarks)

| Optimization | Speedup | Token Savings | Cost Reduction |
|-------------|---------|---------------|----------------|
| **vLLM prefix caching** | **1.51x** | 70% (4,368 tokens) | 31% |
| **DeadContextElimination** | **2.00x** | 66% | 71% |
| **MemoCache (2nd run)** | **1.09x** | 100% (all cached) | 100% |
| **Cumulative (O2+cache+vLLM)** | **1.51-2x** | **70%** | **92%** |

**Annual savings** (100 workflows/day, GPT-4o): **$805/year** (from $875 → $70)

---

## Documentation Structure

### Core Documentation (Start Here)

**1. [ARCHITECTURE.md](ARCHITECTURE.md)** — **Complete system architecture**
- Frontend layer (Python API, CLI)
- Compiler layer (MLIR passes, O0-O3, targets)
- Runtime layer (scheduler, handlers, caching)
- Backend layer (OpenAI, vLLM, Ollama)
- Optimization layer (heuristics, PGO, DSPy)
- **ASCII architecture diagram**

**2. [OPTIMIZATION-GUIDE.md](OPTIMIZATION-GUIDE.md)** — **How optimization works end-to-end**
- Optimization levels (O0, O1, O2, O3)
- Optimization targets (latency, cost, tokens, quality, balanced)
- vLLM integration (prefix caching, priority scheduling)
- DSPy integration (prompt optimization)
- MemoCache (persistent cross-session caching)
- **Decision matrix**: "what should I use for my workload?"

**3. [CHANGELOG.md](CHANGELOG.md)** — **What was built April 7-8, 2026**
- Frontend changes (auto-wiring, typed profiles)
- Compiler changes (new passes, targets)
- Runtime changes (vLLM integration, session tracing)
- Benchmark results summary
- Migration guide (0.1.x → 0.2.0)

**4. [FINDINGS.md](FINDINGS.md)** — **Comprehensive benchmark results**
- vLLM prefix caching (1.51x speedup, 70% cache hit rate)
- Compiler optimizations (2x speedup from dead-context elimination)
- MemoCache effectiveness (100% hit rate on 2nd run)
- Per-pass profiling analysis

**5. [COST-ANALYSIS.md](COST-ANALYSIS.md)** — **Real dollar savings**
- Per-optimization cost breakdown
- Annual savings projections
- Break-even analysis (cloud vs self-hosted)

---

### Theory & Foundations [`pxm/`](pxm/)

What APXM *is* — the formal execution model.

- [**history.md**](pxm/history.md) — How computing history repeats: from von Neumann to agents
- [**foundations.md**](pxm/foundations.md) — The agentic von Neumann bottleneck
- [**aam.md**](pxm/aam.md) — Agent Abstract Machine (Beliefs, Goals, Capabilities)
- [**ais.md**](pxm/ais.md) — Agent Instruction Set (31 typed operations)
- [**compute.md**](pxm/compute.md), [**memory.md**](pxm/memory.md), [**scheduling.md**](pxm/scheduling.md) — Deep dives
- [**vision.md**](pxm/vision.md) — The LLVM-for-agents vision

---

### Implementation Details [`implementation/`](implementation/)

Compiler, runtime, contracts (for contributors).

- [**architecture.md**](implementation/architecture.md) — System architecture index
- [**TODO.md**](implementation/TODO.md) — Master TODO (AIS gaps, substrate gaps, AAM gaps)
- [`ais/`](implementation/ais/) — Per-category operation reference
- [`compiler/`](implementation/compiler/) — MLIR pipeline, passes, artifact format
- [`runtime/`](implementation/runtime/) — Dataflow scheduler, memory hierarchy
- [`internals/`](implementation/internals/) — Wire contracts, graph JSON contract

---

### User Guides [`guides/`](guides/)

Getting started, tutorials, how-tos.

- [**getting-started.md**](guides/getting-started.md) — Install, build, first run
- [**first-graph.md**](guides/first-graph.md) — Write your first workflow
- [**optimization.md**](guides/optimization.md) — Optimization strategies
- [**vllm-integration.md**](guides/vllm-integration.md) — vLLM setup + graph-aware scheduling
- [**dspy-optimization.md**](guides/dspy-optimization.md) — DSPy prompt optimization
- [**backends.md**](guides/backends.md) — Backend setup (OpenAI, Anthropic, vLLM, Ollama)
- [**debugging.md**](guides/debugging.md) — Debugging workflows
- [**multi-agent.md**](guides/multi-agent.md) — Multi-agent orchestration
- [**caching.md**](guides/caching.md) — MemoCache usage

---

### Research Documents [`research/`](research/)

In-depth analysis and strategy.

**Token & Cost Optimization**:
- [**TOKEN-ESTIMATION.md**](research/TOKEN-ESTIMATION.md) — tiktoken integration, cross-model estimation
- [**STRATEGY-TOKEN-SAVING.md**](research/STRATEGY-TOKEN-SAVING.md) — Minimize cost per workflow
- [**PRODUCTION-HEURISTICS.md**](research/PRODUCTION-HEURISTICS.md) — SGLang, Autellix, LangChain, Guidance

**Performance Optimization**:
- [**STRATEGY-PERFORMANCE.md**](research/STRATEGY-PERFORMANCE.md) — Max latency reduction
- [**STRATEGY-QUALITY.md**](research/STRATEGY-QUALITY.md) — Max output correctness

**DSPy Integration**:
- [**DSPY-INTERNALS.md**](research/DSPY-INTERNALS.md) — BootstrapFewShot, MIPROv2 mechanics
- [**DSPY-COMPILER-PASS.md**](research/DSPY-COMPILER-PASS.md) — DSPy as first-class optimization pass

---

### Benchmark Reports [`benchmarks/`](benchmarks/)

Measurement methodology and results.

- [**METHODOLOGY.md**](benchmarks/METHODOLOGY.md) — Three-tier benchmark approach
- [**VLLM-LIVE-RESULTS.md**](benchmarks/VLLM-LIVE-RESULTS.md) — vendor GPU deployment
- [**RESULTS-WEEK1.md**](benchmarks/RESULTS-WEEK1.md) — Compiler pass benchmarks
- [**CACHE-RESULTS.md**](benchmarks/CACHE-RESULTS.md) — MemoCache effectiveness
- [**DSPY-RESULTS.md**](benchmarks/DSPY-RESULTS.md) — DSPy prompt optimization
- [**PASS-PROFILING.md**](benchmarks/PASS-PROFILING.md) — Per-pass firing analysis
- [**LITERATURE-SURVEY.md**](benchmarks/LITERATURE-SURVEY.md) — 76 academic references

---

### Strategy & Planning [`strategy/`](strategy/)

Master plan, design decisions.

- [**11-MASTER-PLAN.md**](strategy/11-MASTER-PLAN.md) — Validation, timeline, next steps
- [**08-OPTIMIZATION-TARGETS.md**](strategy/08-OPTIMIZATION-TARGETS.md) — Multi-objective framework
- [**10-DSPY-INTEGRATION.md**](strategy/10-DSPY-INTEGRATION.md) — DSPy as compiler pass

---

### Design Documents [`design/`](design/)

ADRs and prior art analysis.

- [**tools-cli-design.md**](design/tools-cli-design.md) — CLI architecture
- [**hierarchical-scope-prior-art.md**](design/hierarchical-scope-prior-art.md) — File-tree agents

---

### Applied Projects [`projects/`](projects/)

Concrete applications built on APXM.

- [`codex/`](projects/codex/) — Codex-on-APXM (coding agent reconstruction)
- [`agentmate/`](projects/agentmate/) — AgentMate SDK (Rust + Python frontend)

---

## Architecture Overview

```
┌─────────────────────────────────────────┐
│          Frontend (Python/CLI)          │
│  @compile() → graph.apxm (JSON)         │
└──────────────────┬──────────────────────┘
                   │
                   ▼
┌─────────────────────────────────────────┐
│       Compiler (MLIR + Rust)            │
│  Parser → MLIR IR → 93 Passes → .apxmobj│
│  O0/O1/O2/O3, --target latency/cost/... │
└──────────────────┬──────────────────────┘
                   │
                   ▼
┌─────────────────────────────────────────┐
│        Runtime (Rust executor)          │
│  Artifact → Scheduler → Handlers        │
│  Parallelism, caching, session tracing  │
└──────────────────┬──────────────────────┘
                   │
                   ▼
┌─────────────────────────────────────────┐
│       Backends (LLM Providers)          │
│  OpenAI, Anthropic, vLLM, Ollama        │
│  Graph-aware vLLM: prefix cache, priority│
└─────────────────────────────────────────┘
```

---

## Key Innovations

1. **Compiler-Driven Optimization** — First system to apply LLVM-style optimization passes to LLM orchestration
2. **Graph-Aware vLLM** — Compiler detects shared prefixes, vLLM caches KV blocks (1.51x speedup)
3. **Multi-Target Optimization** — Single graph compiles to 5 different strategies (latency, cost, tokens, quality, balanced)
4. **DSPy as Compiler Pass** — Prompt optimization at compile time, zero runtime cost
5. **MemoCache** — Two-tier persistent caching (L1 in-memory, L2 SQLite)
6. **Session Replay** — Every execution is reproducible from `trace.ndjson`

---

## Comparison to Other Systems

| System | Architecture | Optimization | Key Difference |
|--------|-------------|--------------|----------------|
| **LangChain** | Runtime library | Node-level caching | APXM has compile-time optimization |
| **LangGraph** | Stateful runtime | Checkpointing | APXM is dataflow (stateless nodes) |
| **SGLang** | Runtime server | RadixAttention | APXM integrates as backend + graph-level analysis |
| **DSPy** | Prompt optimizer | MIPROv2 | APXM uses DSPy as compiler pass |

**APXM's unique position**: Only system combining compiler-driven optimization with LLM orchestration.

---

## Reading Paths

### For New Users
1. [getting-started.md](guides/getting-started.md)
2. [first-graph.md](guides/first-graph.md)
3. [OPTIMIZATION-GUIDE.md](OPTIMIZATION-GUIDE.md)
4. [backends.md](guides/backends.md)

### For Researchers
1. [foundations.md](pxm/foundations.md)
2. [aam.md](pxm/aam.md)
3. [ais.md](pxm/ais.md)
4. [ARCHITECTURE.md](ARCHITECTURE.md)
5. [FINDINGS.md](FINDINGS.md)

### For Contributors
1. [ARCHITECTURE.md](ARCHITECTURE.md)
2. [implementation/architecture.md](implementation/architecture.md)
3. [implementation/compiler/](implementation/compiler/)
4. [implementation/TODO.md](implementation/TODO.md)

### For Production Deployments
1. [vllm-integration.md](guides/vllm-integration.md)
2. [OPTIMIZATION-GUIDE.md](OPTIMIZATION-GUIDE.md)
3. [COST-ANALYSIS.md](COST-ANALYSIS.md)
4. [backends.md](guides/backends.md)

---

## External References

- **APXM Research Papers** — Foundational academic papers (v1, v2) on the Agent Program Execution Model
- **Quantum Quill Lyceum** ([skool.com/quantum-quill-lyceum-1116](https://www.skool.com/quantum-quill-lyceum-1116)) — File-tree agent architecture
- **vLLM** — [github.com/vllm-project/vllm](https://github.com/vllm-project/vllm)
- **SGLang** — [github.com/sgl-project/sglang](https://github.com/sgl-project/sglang)
- **DSPy** — [dspy.ai](https://dspy.ai)
- **MLIR** — [mlir.llvm.org](https://mlir.llvm.org)

---

## License

See [LICENSE](../LICENSE) in project root.

---

## Contact

- **Issues**: [github.com/anthropics/apxm/issues](https://github.com/anthropics/apxm/issues)
- **Documentation**: This repository (`docs/`)
- **CLI Help**: `apxm --help` or `/help` in agent sessions

---

**Last Updated**: April 8, 2026
**Version**: 0.2.0
**Commit**: 81b679c
