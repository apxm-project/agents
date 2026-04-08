# APXM System Architecture

**Version**: 0.2.0
**Date**: April 8, 2026
**Status**: Production-ready compiler + runtime with graph-aware vLLM integration

---

## Overview

APXM (Agent Programming eXecution Model) compiles and executes agent workflows. Graphs of AIS (Agent Instruction Set) operations pass through an MLIR compiler that applies optimization passes, then execute on a parallel dataflow scheduler backed by LLM providers.

APXM treats agent workflows as programs. Compiler techniques -- dead code elimination, common subexpression elimination, prompt fusion -- apply directly to LLM orchestration, producing pre-optimized binary artifacts.

```
Python/CLI Frontend --> MLIR Compiler --> Runtime Scheduler --> LLM Backends
     (graphs)       (optimizations)    (parallelism)    (OpenAI, vLLM, etc.)
```

For the graph wire format, see [reference/graph-format.md](reference/graph-format.md).
For configuration details, see [reference/config.md](reference/config.md).

---

## System Layers

### 1. Frontend Layer

Provides user-facing APIs for authoring workflows.

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

**Output**: `.apxm` graph files (JSON-based). See [reference/graph-format.md](reference/graph-format.md).

**Example**:
```python
from apxm import compile

@compile()
def code_review(g):
    code = g.text(value="def foo(): return 42")
    review = g.ask("Review this code: {{code}}")
    g.done(review)
```

For a hands-on walkthrough, see [getting-started/first-graph.md](getting-started/first-graph.md).

---

### 2. Compiler Layer

Transforms graphs into optimized binary artifacts.

**Architecture**:
```
.apxm (JSON) --> Graph Parser --> MLIR IR --> Optimization Passes --> .apxmobj (binary)
```

**Components**:

#### 2.1 Graph Parser (`apxm-graph`)
- Validates `.apxm` files against the AIS contract
- Converts JSON to MLIR `ais.*` operations
- Performs type checking and schema validation

See [implementation/compiler/overview.md](implementation/compiler/overview.md) for internals.

#### 2.2 MLIR Compiler (`apxm-compiler/mlir/`)
- **Custom dialect**: `ais` (Agent Instruction Set)
- **31 operations**: ASK, THINK, REASON, PLAN, SPAWN_AGENT, COMMUNICATE, etc.
- **93 MLIR passes** including 13 APXM-specific optimizations

For the full AIS operation reference, see [pxm/ais.md](pxm/ais.md).
For compiler integration details, see [implementation/compiler-integration.md](implementation/compiler-integration.md).

#### 2.3 Optimization Passes

**O0** (none): Direct graph-to-artifact, no transforms.

**O1** (basic):
- `normalize` -- Canonicalize graph structure
- `build-prompt` -- Construct templates
- `scheduling` -- Reorder for parallelism
- `fuse-ask-ops` -- Merge sequential LLM calls
- `canonicalizer` -- MLIR standard optimizations
- `cse` -- Common subexpression elimination
- `symbol-dce` -- Dead code elimination

**O2** (standard, production default):
- All O1 passes, plus:
- `template-specialization` -- Inline constants
- `dead-context-elimination` -- Remove unused context
- `schema-narrowing` -- Simplify output schemas
- `condense-ops` -- Batch memory operations
- `prompt-canonicalization` -- Extract shared prefixes

**O3** (aggressive):
- All O2 passes, plus:
- `dspy-optimize` -- Prompt quality optimization via DSPy

For per-pass details and profiling data, see [optimization/passes.md](optimization/passes.md).
For the DSPy integration specifics, see [integrations/dspy.md](integrations/dspy.md).

**Per-Target Tuning**:
- `--target latency`: Aggressive fusion, pipelining, speculation
- `--target cost`: Model downgrading, budget enforcement
- `--target tokens`: Dead-context elimination, schema narrowing, LLMLingua
- `--target quality`: Verification injection, refinement loops, best models
- `--target balanced`: Default middle ground

See [optimization/overview.md](optimization/overview.md) for target selection guidance.

#### 2.4 Code Generation (`apxm-artifact`)
- Emits binary artifacts (`.apxmobj`, typically 2--50 KB)
- Header: version, hash, metadata
- Serialized operations with attributes
- Wire format: C++ emitter, Rust parser

See [implementation/compiler/artifact-format.md](implementation/compiler/artifact-format.md).

---

### 3. Runtime Layer

Executes compiled artifacts with parallelism and caching.

**Architecture**:
```
.apxmobj --> Artifact Loader --> Scheduler --> Operation Handlers --> Results
                                    |
                              LLM Backends
                         (OpenAI, vLLM, Ollama, mock)
```

**Components**:

#### 3.1 Executor (`apxm-runtime/src/executor/`)
- **RuntimeExecutor**: Main execution engine
- **Parallel scheduler**: Topological sort + dependency tracking
- **Session management**: Live tracing, results, metrics

See [implementation/runtime/dataflow-scheduler.md](implementation/runtime/dataflow-scheduler.md).

#### 3.2 Operation Handlers (`handlers/`)
- `llm_ops.rs`: ASK, THINK, REASON, PLAN, REFLECT, VERIFY
- `flow_ops.rs`: CONST_STR, PRINT, YIELD, PARAM
- `agent_ops.rs`: SPAWN_AGENT, COMMUNICATE
- `memory_ops.rs`: QUERY_MEMORY, UPDATE_MEMORY
- `control_ops.rs`: BRANCH_ON_VALUE, SWITCH, GUARD

Each handler implements the `OperationHandler` trait:
```rust
pub trait OperationHandler {
    async fn execute(&self, node: &Node, inputs: &[Value]) -> Result<Value>;
}
```

For LLM operation semantics, see [implementation/ais/llm-ops.md](implementation/ais/llm-ops.md).
For multi-agent execution, see [implementation/runtime/multi-agent.md](implementation/runtime/multi-agent.md).

#### 3.3 MemoCache (`cache.rs`)
- **L1**: In-memory DashMap (10,000 entries)
- **L2**: SQLite (`~/.apxm/cache/cache.db`)
- **TTL per operation**:
  - ASK: 1 hour
  - THINK: 24 hours
  - REASON: 7 days
- **Deterministic only**: `temperature=0.0`

See [implementation/runtime/memory-hierarchy.md](implementation/runtime/memory-hierarchy.md) and [guides/caching.md](guides/caching.md).

#### 3.4 Session Output (`~/.apxm/sessions/<id>/`)
```
session/
+-- manifest.json         # Execution metadata
+-- input.apxm            # Copy of input graph
+-- trace.ndjson          # Live event stream
+-- live.json             # Current progress (atomic updates)
+-- results.json          # All node outputs
+-- metrics.json          # Execution metrics
+-- node_statuses.json    # Per-node status
+-- nodes/<id>_<name>/    # Per-node workspace
    +-- CLAUDE.md         # Agent context
    +-- output.json       # Final output
    +-- trace.ndjson      # Node event trace
```

See [implementation/runtime/sessions.md](implementation/runtime/sessions.md) and [implementation/runtime/observability.md](implementation/runtime/observability.md).

---

### 4. Backend Layer

Adapts LLM providers to the runtime's unified interface.

**Supported Backends**:

| Backend | Type | Protocol | Features |
|---------|------|----------|----------|
| **OpenAI** | Cloud | OpenAI API | GPT-4o, GPT-4.5, function calling |
| **Anthropic** | Cloud | Messages API | Claude Opus/Sonnet 4.6 |
| **vLLM** | Local | OpenAI-compatible | Prefix caching, priority hints, KV pinning |
| **Ollama** | Local | OpenAI-compatible | Llama 3.3, Qwen 2.5 |
| **Mock** | Test | N/A | Fixed latency for benchmarking |

See [guides/backends.md](guides/backends.md) for setup instructions.

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

See [reference/config.md](reference/config.md) for the full configuration schema.

#### 4.1 vLLM Graph-Aware Integration

APXM extends vLLM with graph-level awareness for three capabilities:

- **Prefix caching**: Automatic KV cache reuse (70--95% hit rate)
- **Priority scheduling**: Critical-path nodes receive priority 0
- **KV pinning**: Pin upstream KV blocks for downstream reuse

**Integration points**:
1. Compiler: `prompt-canonicalization` pass detects shared prefixes
2. Backend: `GraphAwareVllmBackend` injects priority/reuse hints
3. vLLM: Scheduler consumes hints for optimization

**Example hints**:
```rust
RequestHints {
    priority: 0,                     // Critical path
    reuse_group: "code-context",     // Shared prefix group
    pin_policy: PinPolicy::prefix(300_000),  // 5-minute TTL
}
```

See [integrations/vllm.md](integrations/vllm.md) for the full integration guide.

---

### 5. Optimization Layer

Combines heuristics and profile-guided optimization to tune compiled artifacts.

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
2. **Analyze**: Session traces yield execution statistics
3. **Recompile**: `apxm compile graph.apxm --profile ~/.apxm/sessions/<id>`
4. **Result**: Heuristics tuned to actual execution patterns

**Metrics collected**:
- Per-node latency, token counts, cache hit rates
- Quality scores (when ground truth is available)
- Model performance (accuracy, confidence)

#### 5.3 DSPy Integration

DSPy provides learned prompt optimization at two stages:

**Mode 1 -- Compile-time** (DSPy as compiler pass):
```bash
apxm compile graph.apxm -O3 --dspy --dspy-training-data examples.json
```
- Optimizes templates before artifact generation
- Zero runtime cost; optimized prompts embed in `.apxmobj`

**Mode 2 -- Runtime** (session-based learning):
```bash
apxm execute graph.apxm --dspy=auto
```
- Uses execution history as training data
- Refines prompts across sessions

**Optimizers**:
- `LabeledFewShot`: 3--5 examples, fast, +10--15% quality
- `BootstrapFewShot`: Mines successful traces, +20--30% quality
- `MIPROv2`: Bayesian optimization, +30--40% quality (15--30 min)

See [integrations/dspy.md](integrations/dspy.md) for setup and usage.

---

## Benchmarks

> For benchmark results, see [benchmarks/results/2026-04-08.md](benchmarks/results/2026-04-08.md).

For benchmark methodology, see [benchmarks/methodology/compiler.md](benchmarks/methodology/compiler.md) and [benchmarks/methodology/vllm.md](benchmarks/methodology/vllm.md).

---

## Data Flow Diagram

```
+----------------------------------------------------------------+
|                     FRONTEND (Python/CLI)                       |
|  +-------------+  +--------------+  +------------------------+ |
|  | @compile()  |->| GraphRecorder|->| graph.apxm (JSON)      | |
|  +-------------+  +--------------+  +------------------------+ |
+-------------------------------+--------------------------------+
                                |
                                v
+----------------------------------------------------------------+
|                   COMPILER (MLIR + Rust)                        |
|  +----------+  +---------+  +--------------+  +-------------+ |
|  | Parser   |->| MLIR IR |->| 93 Passes    |->| .apxmobj    | |
|  | (JSON)   |  | (ais.*) |  | (O0/O1/O2/O3)|  | (binary)    | |
|  +----------+  +---------+  +--------------+  +-------------+ |
|                                     |                          |
|                                     v                          |
|                         +-----------------------+              |
|                         | Optimization Passes   |              |
|                         | - DeadContextElim     |              |
|                         | - FuseAskOps          |              |
|                         | - PromptCanonical     |              |
|                         | - CSE                 |              |
|                         +-----------------------+              |
+-------------------------------+--------------------------------+
                                |
                                v
+----------------------------------------------------------------+
|                    RUNTIME (Rust executor)                      |
|  +--------------+  +------------+  +--------------------------+|
|  | Artifact     |->| Scheduler  |->| Operation Handlers       ||
|  | Loader       |  | (parallel) |  | (ASK, THINK, SPAWN, ...) ||
|  +--------------+  +------------+  +--------------------------+|
|                         |                    |                  |
|                         v                    v                  |
|                   +----------+        +----------+             |
|                   |MemoCache |        | Session  |             |
|                   |(L1 + L2) |        | Tracer   |             |
|                   +----------+        +----------+             |
+-------------------------------+--------------------------------+
                                |
                                v
+----------------------------------------------------------------+
|                 BACKENDS (LLM Providers)                        |
|  +----------+  +----------+  +----------+  +----------+       |
|  | OpenAI   |  | Anthropic|  | vLLM     |  | Ollama   |       |
|  | (GPT-4o) |  | (Claude) |  | (local)  |  | (local)  |       |
|  +----------+  +----------+  +----------+  +----------+       |
|                                |                               |
|                                v                               |
|                    +-----------------------+                   |
|                    | vLLM Graph-Aware      |                   |
|                    | - Prefix caching      |                   |
|                    | - Priority scheduling |                   |
|                    | - KV pinning          |                   |
|                    +-----------------------+                   |
+----------------------------------------------------------------+
```

---

## Production Deployment

### Hardware Requirements

**For local vLLM**:
- **GPU**: vendor GPU (192GB HBM3) or NVIDIA A100/H100
- **Memory**: 32GB+ RAM
- **Storage**: 100GB for models + cache

**For cloud-only**: No GPU required.

See [getting-started/installation.md](getting-started/installation.md) for full setup instructions.

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

See [guides/debugging.md](guides/debugging.md) for troubleshooting session output.

---

## Architecture Principles

1. **Separation of concerns**: Frontend (authoring) | Compiler (optimization) | Runtime (execution)
2. **Data-driven optimization**: Compiler passes driven by heuristics + profiles, not hardcoded policies
3. **Zero runtime overhead**: Optimization at compile time; artifacts are pre-optimized binaries
4. **Composable backends**: Adapter pattern allows swapping LLM providers without graph changes
5. **Observable execution**: Every run produces session traces for debugging and profile-guided re-optimization
6. **Deterministic replay**: Sessions replay exactly from `trace.ndjson`

---

## Comparison to Other Systems

| System | Architecture | Optimization | Key Difference |
|--------|-------------|--------------|----------------|
| **LangChain** | Runtime library | Node-level caching | APXM optimizes at compile time |
| **LangGraph** | Stateful runtime | Checkpointing | APXM uses stateless dataflow nodes |
| **SGLang** | Runtime server | RadixAttention | APXM integrates as backend, adds graph-level analysis |
| **DSPy** | Prompt optimizer | MIPROv2, BootstrapFewShot | APXM uses DSPy as a compiler pass |
| **Semantic Kernel** | Runtime library | Native function calling | APXM has first-class agent operations |

APXM occupies a unique position: the only system combining compiler-driven optimization (in the LLVM tradition) with LLM orchestration.

---

## References

### Foundations
- [pxm/foundations.md](pxm/foundations.md) -- Theoretical model (PXM)
- [pxm/ais.md](pxm/ais.md) -- Agent Instruction Set specification
- [pxm/aam.md](pxm/aam.md) -- Abstract Agent Machine

### Implementation
- [implementation/compiler/overview.md](implementation/compiler/overview.md) -- Compiler internals
- [implementation/runtime/dataflow-scheduler.md](implementation/runtime/dataflow-scheduler.md) -- Scheduler design
- [implementation/architecture.md](implementation/architecture.md) -- Crate-level architecture

### Guides
- [getting-started/installation.md](getting-started/installation.md) -- Installation
- [getting-started/first-graph.md](getting-started/first-graph.md) -- First graph tutorial
- [guides/backends.md](guides/backends.md) -- Backend configuration
- [guides/multi-agent.md](guides/multi-agent.md) -- Multi-agent workflows
- [guides/debugging.md](guides/debugging.md) -- Debugging and session replay

### Optimization
- [optimization/overview.md](optimization/overview.md) -- Optimization strategy
- [optimization/passes.md](optimization/passes.md) -- Pass reference
- [integrations/vllm.md](integrations/vllm.md) -- vLLM graph-aware integration
- [integrations/dspy.md](integrations/dspy.md) -- DSPy prompt optimization

### Design
- [design/agent-ontology.md](design/agent-ontology.md) -- Agent type taxonomy
- [design/agent-profiles.md](design/agent-profiles.md) -- Profile system
- [design/hierarchical-aam.md](design/hierarchical-aam.md) -- Hierarchical AAM
- [design/agentic-os.md](design/agentic-os.md) -- OS-level agent abstractions

### Reference
- [reference/config.md](reference/config.md) -- Configuration schema
- [reference/graph-format.md](reference/graph-format.md) -- Graph file format
- [CLAUDE.md](../CLAUDE.md) -- CLI command reference
