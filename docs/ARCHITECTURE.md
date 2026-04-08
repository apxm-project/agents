# APXM System Architecture

**Version**: 0.2.0
**Date**: April 8, 2026
**Status**: Production-ready compiler + runtime with graph-aware vLLM integration

---

## Overview

APXM (Agent Programming eXecution Model) is a **compiler and dataflow runtime for agent workflows**. Graphs of AIS (Agent Instruction Set) operations compile through MLIR to optimized artifacts, then execute on a parallel scheduler with LLM backends.

**Key innovation**: APXM treats agent workflows as programs that can be compiled, optimized, and executed efficiently — applying compiler techniques (dead code elimination, common subexpression elimination, fusion) to LLM orchestration.

```
Python/CLI Frontend → MLIR Compiler → Runtime Scheduler → LLM Backends
     (graphs)       (optimizations)    (parallelism)    (OpenAI, vLLM, etc.)
```

---

## System Layers

### 1. Frontend Layer

**Purpose**: User-facing APIs for authoring workflows

**Components**:
- **Python API** (`apxm-frontend/python/apxm/`)
  - `GraphRecorder`: Context manager for graph construction
  - Auto-wiring: Infers data dependencies from context usage
  - Operations: `g.ask()`, `g.think()`, `g.reason()`, `g.plan()`, `g.spawn()`, etc.
  - Compilation: `@compile()` decorator or `apxm.compile_graph()`

- **CLI** (`bin/apxm`, `tools/apxm_cli.py`)
  - Graph validation, analysis, compilation, execution
  - Backend management (`apxm backend add/list/test`)
  - Session replay (`apxm replay <session-id>`)

**Output**: `.apxm` graph files (JSON-based)

**Example**:
```python
from apxm import compile

@compile()
def code_review(g):
    code = g.text(value="def foo(): return 42")
    review = g.ask("Review this code: {{code}}")
    g.done(review)
```

---

### 2. Compiler Layer

**Purpose**: Transform graphs → optimized artifacts

**Architecture**:
```
.apxm (JSON) → Graph Parser → MLIR IR → Optimization Passes → .apxmobj (binary)
```

**Components**:

#### 2.1 Graph Parser (`apxm-graph`)
- Validates `.apxm` against AIS contract
- Converts JSON → MLIR `ais.*` operations
- Type checking, schema validation

#### 2.2 MLIR Compiler (`apxm-compiler/mlir/`)
- **Custom dialect**: `ais` (Agent Instruction Set)
- **31 operations**: ASK, THINK, REASON, PLAN, SPAWN_AGENT, COMMUNICATE, etc.
- **93 MLIR passes** including 13 APXM-specific optimizations

#### 2.3 Optimization Passes

**O0** (no optimization): Direct graph → artifact

**O1** (basic):
- `normalize` — Canonicalize graph structure
- `build-prompt` — Construct templates
- `scheduling` — Reorder for parallelism
- `fuse-ask-ops` — Merge sequential LLM calls
- `canonicalizer` — MLIR standard optimizations
- `cse` — Common subexpression elimination
- `symbol-dce` — Dead code elimination

**O2** (standard, production):
- All O1 passes +
- `template-specialization` — Inline constants
- `dead-context-elimination` — Remove unused context
- `schema-narrowing` — Simplify output schemas
- `condense-ops` — Batch memory operations
- `prompt-canonicalization` — Extract shared prefixes

**O3** (aggressive):
- All O2 passes +
- `dspy-optimize` — Prompt quality optimization via DSPy

**Per-Target Tuning**:
- `--target latency`: Aggressive fusion, pipelining, speculation
- `--target cost`: Model downgrading, budget enforcement
- `--target tokens`: Dead-context elimination, schema narrowing, LLMLingua
- `--target quality`: Verification injection, refinement loops, best models
- `--target balanced`: Default middle ground

#### 2.4 Code Generation (`apxm-artifact`)
- MLIR → binary artifact (`.apxmobj`)
- Header: version, hash, metadata
- Serialized operations with attributes
- Wire format: C++ emitter → Rust parser

**Output**: `.apxmobj` compiled artifact (2-50 KB typical)

---

### 3. Runtime Layer

**Purpose**: Execute compiled artifacts with parallelism + caching

**Architecture**:
```
.apxmobj → Artifact Loader → Scheduler → Operation Handlers → Results
                                  ↓
                            LLM Backends
                       (OpenAI, vLLM, Ollama, mock)
```

**Components**:

#### 3.1 Executor (`apxm-runtime/src/executor/`)
- **RuntimeExecutor**: Main execution engine
- **Parallel scheduler**: Topological sort + dependency tracking
- **Session management**: Live tracing, results, metrics

#### 3.2 Operation Handlers (`handlers/`)
- `llm_ops.rs`: ASK, THINK, REASON, PLAN, REFLECT, VERIFY
- `flow_ops.rs`: CONST_STR, PRINT, YIELD, PARAM
- `agent_ops.rs`: SPAWN_AGENT, COMMUNICATE
- `memory_ops.rs`: QUERY_MEMORY, UPDATE_MEMORY
- `control_ops.rs`: BRANCH_ON_VALUE, SWITCH, GUARD

Each handler implements `OperationHandler` trait:
```rust
pub trait OperationHandler {
    async fn execute(&self, node: &Node, inputs: &[Value]) -> Result<Value>;
}
```

#### 3.3 MemoCache (`cache.rs`)
- **L1**: In-memory DashMap (10,000 entries)
- **L2**: SQLite (`~/.apxm/cache/cache.db`)
- **TTL per operation**:
  - ASK: 1 hour
  - THINK: 24 hours
  - REASON: 7 days
- **Deterministic only**: `temperature=0.0`

#### 3.4 Session Output (`~/.apxm/sessions/<id>/`)
```
session/
├── manifest.json         # Execution metadata
├── input.apxm            # Copy of input graph
├── trace.ndjson          # Live event stream
├── live.json             # Current progress (atomic updates)
├── results.json          # All node outputs
├── metrics.json          # Execution metrics
├── node_statuses.json    # Per-node status
└── nodes/<id>_<name>/    # Per-node workspace
    ├── CLAUDE.md         # Agent context
    ├── output.json       # Final output
    └── trace.ndjson      # Node event trace
```

---

### 4. Backend Layer

**Purpose**: Adapter layer for LLM providers

**Supported Backends**:

| Backend | Type | Protocol | Features |
|---------|------|----------|----------|
| **OpenAI** | Cloud | OpenAI API | GPT-4o, GPT-4.5, function calling |
| **Anthropic** | Cloud | Messages API | Claude Opus/Sonnet 4.6 |
| **vLLM** | Local | OpenAI-compatible | Prefix caching, priority hints, KV pinning |
| **Ollama** | Local | OpenAI-compatible | Llama 3.3, Qwen 2.5 |
| **Mock** | Test | N/A | Fixed latency for benchmarking |

**Configuration** (`~/.apxm/config.toml`):
```toml
[[backends]]
name = "vllm-local"
type = "local"
protocol = "openai"
endpoint = "http://localhost:8000/v1"

[[backends.models]]
id = "Qwen/Qwen2.5-7B-Instruct"
context_window = 32768
supports_functions = true
```

#### 4.1 vLLM Graph-Aware Integration

**Capabilities**:
- **Prefix caching**: Automatic KV cache reuse (70-95% hit rate)
- **Priority scheduling**: Critical path gets priority 0
- **KV pinning**: Pin upstream blocks for downstream reuse
- **Continuous batching**: Multiple graphs execute concurrently

**Performance**:
- **1.51x speedup** on fan-out patterns (shared prefix)
- **70% cache hit rate** on multi-perspective analysis
- **4,368 tokens saved** from redundant prefill

**Integration points**:
1. Compiler: `prompt-canonicalization` pass detects shared prefixes
2. Backend: `GraphAwareVllmBackend` injects priority/reuse hints
3. vLLM: Scheduler uses hints for optimization

**Example hints**:
```rust
RequestHints {
    priority: 0,                     // Critical path
    reuse_group: "code-context",     // Shared prefix group
    pin_policy: PinPolicy::prefix(300_000),  // 5-minute TTL
}
```

---

### 5. Optimization Layer

**Purpose**: Heuristics + profile-guided optimization

**Components**:

#### 5.1 Heuristics Engine (`apxm-compiler/src/passes/heuristics.rs`)

**Per-target configuration**:

| Setting | Latency | Cost | Tokens | Quality |
|---------|---------|------|--------|---------|
| max_fused_template_tokens | 32,000 | 5,000 | 2,000 | 0 (no fusion) |
| max_context_tokens | 128K | 16K | 8K | 90% of window |
| enable_quality_guard | false | true | true | true |
| speculation_enabled | true | false | false | false |

#### 5.2 Profile-Guided Optimization (PGO)

**Workflow**:
1. **Collect profiles**: `apxm execute graph.apxm --emit-session`
2. **Analyze**: Session traces → execution statistics
3. **Recompile**: `apxm compile graph.apxm --profile ~/.apxm/sessions/<id>`
4. **Benefit**: Heuristics tuned to actual execution patterns

**Metrics collected**:
- Per-node latency, token counts, cache hit rates
- Quality scores (if ground truth available)
- Model performance (accuracy, confidence)

#### 5.3 DSPy Integration

**Purpose**: Learned prompt optimization

**Mode 1 — Compile-time** (DSPy as compiler pass):
```bash
apxm compile graph.apxm -O3 --dspy --dspy-training-data examples.json
```
- Optimizes templates before artifact generation
- Zero runtime cost
- Embeds optimized prompts in `.apxmobj`

**Mode 2 — Runtime** (session-based learning):
```bash
apxm execute graph.apxm --dspy=auto
```
- Uses execution history as free training data
- Refines prompts across sessions

**Optimizers**:
- `LabeledFewShot`: Add 3-5 examples (fast, +10-15% quality)
- `BootstrapFewShot`: Mine successful traces (+20-30% quality)
- `MIPROv2`: Bayesian optimization (+30-40% quality, 15-30 min)

**Expected improvement**: **+20-40% accuracy** on structured tasks

---

## Data Flow Diagram

```
┌─────────────────────────────────────────────────────────────────┐
│                     FRONTEND (Python/CLI)                        │
│  ┌─────────────┐  ┌──────────────┐  ┌────────────────────────┐ │
│  │ @compile()  │→ │ GraphRecorder│→ │ graph.apxm (JSON)      │ │
│  └─────────────┘  └──────────────┘  └────────────────────────┘ │
└──────────────────────────────┬──────────────────────────────────┘
                               │
                               ▼
┌─────────────────────────────────────────────────────────────────┐
│                   COMPILER (MLIR + Rust)                         │
│  ┌──────────┐  ┌─────────┐  ┌──────────────┐  ┌─────────────┐ │
│  │ Parser   │→ │ MLIR IR │→ │ 93 Passes    │→ │ .apxmobj    │ │
│  │ (JSON)   │  │ (ais.*) │  │ (O0/O1/O2/O3)│  │ (binary)    │ │
│  └──────────┘  └─────────┘  └──────────────┘  └─────────────┘ │
│                                      │                           │
│                                      ▼                           │
│                          ┌───────────────────────┐              │
│                          │ Optimization Passes   │              │
│                          │ - DeadContextElim     │              │
│                          │ - FuseAskOps          │              │
│                          │ - PromptCanonical     │              │
│                          │ - CSE                 │              │
│                          └───────────────────────┘              │
└──────────────────────────────┬──────────────────────────────────┘
                               │
                               ▼
┌─────────────────────────────────────────────────────────────────┐
│                    RUNTIME (Rust executor)                       │
│  ┌──────────────┐  ┌────────────┐  ┌──────────────────────────┐│
│  │ Artifact     │→ │ Scheduler  │→ │ Operation Handlers       ││
│  │ Loader       │  │ (parallel) │  │ (ASK, THINK, SPAWN, ...) ││
│  └──────────────┘  └────────────┘  └──────────────────────────┘│
│                          │                    │                  │
│                          ▼                    ▼                  │
│                    ┌──────────┐        ┌──────────┐             │
│                    │MemoCache │        │ Session  │             │
│                    │(L1 + L2) │        │ Tracer   │             │
│                    └──────────┘        └──────────┘             │
└──────────────────────────────┬──────────────────────────────────┘
                               │
                               ▼
┌─────────────────────────────────────────────────────────────────┐
│                 BACKENDS (LLM Providers)                         │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌──────────┐       │
│  │ OpenAI   │  │ Anthropic│  │ vLLM     │  │ Ollama   │       │
│  │ (GPT-4o) │  │ (Claude) │  │ (local)  │  │ (local)  │       │
│  └──────────┘  └──────────┘  └──────────┘  └──────────┘       │
│                                │                                │
│                                ▼                                │
│                    ┌───────────────────────┐                   │
│                    │ vLLM Graph-Aware      │                   │
│                    │ - Prefix caching      │                   │
│                    │ - Priority scheduling │                   │
│                    │ - KV pinning          │                   │
│                    └───────────────────────┘                   │
└─────────────────────────────────────────────────────────────────┘
```

---

## Key Results (April 7-8, 2026 Benchmarks)

| Optimization | Speedup | Token Savings | Cost Reduction | Benchmark |
|-------------|---------|---------------|----------------|-----------|
| **vLLM prefix caching** | **1.51x** | 70% (4,368 tokens) | 31% | shared_prefix_fanout |
| **DeadContextElimination** | **2.00x** | 66% | 71% | dead_context_stress |
| **MemoCache (2nd run)** | **1.09x** | 100% (all cached) | 100% | cache_test |
| **CSE** | **1.15x** | 30% | 12.5% | cse_stress |
| **Priority scheduling** | **1.12x** | N/A | N/A | mixed_priority |

**Cumulative** (O2 + MemoCache + vLLM): **92% cost reduction**, **1.51-2x speedup**

---

## Production Deployment

### Hardware Requirements

**For local vLLM**:
- **GPU**: vendor GPU (192GB HBM3) or NVIDIA A100/H100
- **Memory**: 32GB+ RAM
- **Storage**: 100GB for models + cache

**For cloud-only**: No GPU required

### Configuration

```bash
# Install APXM
apxm install  # Sets up conda env + MLIR

# Add backends
apxm backend add openai --type cloud --protocol openai --api-key sk-...
apxm backend add vllm-local --type local --protocol openai --endpoint http://localhost:8000

# Add models
apxm backend add-model vllm-local Qwen/Qwen2.5-7B-Instruct

# Test
apxm backend test
```

### Execution

```bash
# Compile
apxm compile workflow.apxm -O2 -o workflow.apxmobj

# Execute
apxm run workflow.apxmobj --emit-session --emit-metrics metrics.json

# Replay
apxm replay ~/.apxm/sessions/<id>
```

---

## Architecture Principles

1. **Separation of concerns**: Frontend (authoring) | Compiler (optimization) | Runtime (execution)
2. **Data-driven optimization**: Compiler passes driven by heuristics + profiles, not hardcoded policies
3. **Zero runtime overhead**: Optimization happens at compile time, artifacts are pre-optimized binaries
4. **Composable backends**: Adapter pattern allows swapping LLM providers without graph changes
5. **Observable execution**: Every run produces session traces for debugging and profile-guided re-optimization
6. **Deterministic replay**: Sessions can be replayed exactly from trace.ndjson

---

## Comparison to Other Systems

| System | Architecture | Optimization | Key Difference |
|--------|-------------|--------------|----------------|
| **LangChain** | Runtime library | Node-level caching | APXM has compile-time optimization |
| **LangGraph** | Stateful runtime | Checkpointing | APXM is dataflow (stateless nodes) |
| **SGLang** | Runtime server | RadixAttention | APXM integrates as backend, adds graph-level analysis |
| **DSPy** | Prompt optimizer | MIPROv2, BootstrapFewShot | APXM uses DSPy as compiler pass |
| **Semantic Kernel** | Runtime library | Native function calling | APXM has first-class agent ops |

**APXM's unique position**: Only system that combines compiler-driven optimization (like LLVM) with LLM orchestration.

---

## References

- [CLAUDE.md](../CLAUDE.md) — CLI reference
- [docs/implementation/](implementation/) — Compiler internals
- [docs/pxm/](pxm/) — Theoretical foundations
- [docs/guides/](guides/) — User guides
- [FINDINGS.md](FINDINGS.md) — Benchmark results
- [OPTIMIZATION-GUIDE.md](OPTIMIZATION-GUIDE.md) — Optimization deep-dive
