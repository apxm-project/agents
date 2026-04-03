# AgentMate: Strategic Assessment & Vision

## Context

AgentMate (`external/agentmate`) is positioned as "the frontend SDK for APXM." The question: **is it worth continuing development?**

This document is a clear-eyed audit of what exists, what duplicates APXM, what's uniquely valuable, and the recommended path forward.

---

## What AgentMate Actually Is Today

**13 Rust crates + 1 Python package spanning two distinct layers:**

### Layer 1: Python Graph DSL (THE CROWN JEWEL)
Location: `crates/am-py/python/agentmate/graph/`

| File | What it does | Unique value? |
|------|-------------|---------------|
| `ir.py` | `ApxmGraph`, `GraphNode`, `GraphEdge`, `Parameter` + validation | **YES** — mirrors Rust types perfectly |
| `proxy.py` | `GraphRecorder` (builder), `NodeRef` with `>>` / `|` operators | **YES** — this is the core UX innovation |
| `module.py` | `FlowModule` (nn.Module analog), composable sub-graphs | **YES** — enables reusable workflow components |
| `execution.py` | `CompiledFlow` — validate → compile → execute via CLI subprocess | **YES** — bridges Python to APXM backend |
| `decorators.py` | `@compile` decorator for function-to-graph capture | **YES** — lowest friction API |
| `constants.py` | AIS op names + attribute keys, with PyO3 native fallback | **YES** — single source of truth bridge |
| `config.py` | `AgentConfig` (model, provider, temperature, system_prompt) | **YES** — per-node LLM configuration |

**30+ AIS operations** have typed Python methods in `GraphRecorder`: `ask()`, `think()`, `reason()`, `plan()`, `reflect()`, `verify()`, `invoke()`, `branch()`, `switch_()`, `wait_all()`, `merge()`, `fence()`, `communicate()`, `guard()`, `claim()`, `pause()`, `resume()`, `update_goal()`, `execute()`, `print_()`, `jump()`, `loop_start()`, `loop_end()`, `return_()`, `flow_call()`, `try_catch()`, `err()`, `agent()`, `const_()`, `yield_()`, `checkpoint()`, `handoff()`, `handoff_when()`, `input_guardrail()`, `output_guardrail()`.

**Missing ops**: `SPAWN_AGENT`, `DELEGATE`, `NEGOTIATE`, `REGISTER_CAPABILITY`, `AUTONOMOUS`, `NOP`, `IDENTITY`.

### Layer 2: Independent Rust Agent Framework (THE DUPLICATION PROBLEM)

| Crate | What it does | Overlaps with APXM? |
|-------|-------------|---------------------|
| `am-core` | Message, ToolCall, LlmProvider, CompletionResponse | **YES** — apxm-core has Value, AISOperationType, all types |
| `am-agents` | AMAgent, AgentBuilder, agent loop, tool execution | **YES** — apxm-runtime executor + handlers do this |
| `am-tools` | Tool trait, ToolRegistry, bash/read/write/search | **YES** — apxm-tools has the same |
| `am-sandbox` | Landlock, seccomp, sandboxed execution | **YES** — apxm-sandbox |
| `am-config` | Layered config, prompt templates, schema | **PARTIAL** — apxm has config.toml + agents.toml |
| `am-cli` | Chat, exec, models, sessions | **YES** — apxm-cli does all this |
| `am-tui` | Terminal UI, streaming display | **PARTIAL** — APXM has session tracing |
| `am-rag` | Chunking, embeddings, vector store | **NO** — APXM has QMEM/UMEM but no RAG pipeline |
| `am-documents` | PDF, Word, Excel, CSV parsing | **NO** — APXM doesn't have this |
| `am-mcp` | MCP client + server | **YES** — apxm-server has MCP |
| `am-macros` | Proc macros for flows | **UNIQUE** — APXM doesn't have this |
| `am-skills` | Skill packages | **YES** — APXM has .agents/skills/ |

**9 of 12 Rust crates substantially overlap with APXM crates.** The Cargo.toml already patches against APXM git deps (`apxm-core`, `apxm-graph`, `apxm-runtime`, etc.), confirming the dependency exists but the crate boundaries are duplicated.

---

## The Hard Truth

AgentMate is essentially **two projects bolted together**:

1. **A Python graph DSL** (am-py) — genuinely excellent, fills a real gap
2. **A standalone agent framework** (am-core through am-cli) — reimplements what APXM already does

The Rust layer (layer 2) creates a **competing runtime** rather than leveraging APXM's runtime. An `AMAgent.run("task")` call goes through am-agents → am-core → LLM provider directly, bypassing APXM's compiler, scheduler, and optimizations entirely.

The Python layer (layer 1) correctly generates `.apxm` JSON and shells out to the APXM CLI. This is the right architecture.

---

## Recommendation: Lean Frontend (Option A)

**Strip agentmate to its unique value: the Python graph package.**

### What to KEEP
```
crates/am-py/python/agentmate/
├── __init__.py          # Package entry
├── graph/
│   ├── __init__.py      # Public API
│   ├── ir.py            # ApxmGraph, GraphNode, GraphEdge, Parameter, validation
│   ├── proxy.py         # GraphRecorder, NodeRef (the builder)
│   ├── module.py        # FlowModule (composable sub-graphs)
│   ├── execution.py     # CompiledFlow (CLI subprocess bridge)
│   ├── decorators.py    # @compile decorator
│   ├── constants.py     # AIS ops + attribute keys
│   ├── config.py        # AgentConfig (per-node LLM settings)
│   └── utils.py         # DAG cycle detection
├── agent.py             # Simple Agent class (high-level wrapper)
├── tools.py             # Tool/FunctionTool decorator (@tool)
├── supervisor.py        # Supervisor (multi-agent orchestration)
├── codelet.py           # Codelet abstraction
├── events.py            # Event streaming types
├── providers.py         # Provider registry
├── prompts.py           # PromptTemplate, FewShotExample
├── guardrails.py        # Guardrail abstractions
├── hitl.py              # Human-in-the-loop (ApprovalGate)
├── mcp.py               # MCP server config
└── context.py           # RunContext
```

Plus: `examples/`, `tests/`, `pyproject.toml`, `docs/`

### What to DROP (or archive)
All Rust crates: `am-core`, `am-agents`, `am-tools`, `am-sandbox`, `am-config`, `am-cli`, `am-tui`, `am-rag`, `am-documents`, `am-mcp`, `am-macros`, `am-skills`

**Why**: These are building a second agent runtime. APXM already has all of this, and it's better — it has the MLIR compiler, dataflow scheduler, and optimizer.

### What to ADD
1. **Missing AIS ops** in `proxy.py`: `spawn_agent()`, `delegate()`, `negotiate()`, `register_capability()`, `autonomous()`, `nop()`, `identity()`
2. **Direct APXM CLI integration**: Fix `CompiledFlow._fallback_subprocess()` to use `dekk apxm execute` instead of bare `apxm`
3. **pyproject.toml as standalone** (no maturin/PyO3 needed — pure Python, falls back gracefully when native unavailable)

### The vLLM Analogy

vLLM's architecture:
```
Python API (user-facing) → C++/CUDA kernel (execution)
```

APXM + AgentMate (lean):
```
Python API (agentmate.graph) → .apxm JSON → APXM compiler (MLIR) → .apxmobj → APXM runtime
```

The Python package is the **frontend**. APXM is the **backend**. No Rust code needed in agentmate — it's pure Python producing JSON that the existing APXM pipeline consumes.

---

## Alternative: Keep as Full SDK (Option B)

If you want agentmate as a **standalone product** (not just an APXM frontend):

- Refactor am-agents/am-core to use `apxm-runtime` directly (not reimplement)
- am-tools wraps apxm-tools (not duplicates)
- am-sandbox delegates to apxm-sandbox
- This is 2-3 months of integration work to de-duplicate

**Risk**: You're maintaining two codebases that need to stay in sync.

---

## Effort Assessment

| Path | Effort | Maintenance burden | Value delivered |
|------|--------|-------------------|-----------------|
| **A: Lean frontend** | 1-2 weeks | Low (pure Python, no Rust) | High — agents can easily write Python graphs |
| **B: Full SDK** | 2-3 months | High (13 crates to keep in sync) | Medium — duplicates what APXM already does |
| **C: Status quo** | 0 | Very high (two diverging runtimes) | Diminishing |

---

## Bottom Line

**The Python graph DSL is worth keeping. It's genuinely good.** The `FlowModule`/`GraphRecorder`/`NodeRef` pattern is elegant, covers 30+ AIS ops, validates against the APXM contract, and produces correct `.apxm` JSON. The showcase examples (code_architect, research_pipeline) demonstrate real value.

**The 12 Rust crates are not worth keeping.** They reimplement what APXM already does (and does better, with compiler optimizations). The maintenance cost of keeping two Rust agent runtimes in sync will drain effort from the actual product.

**Recommended next step**: Decide whether Option A (lean frontend) is the right direction. If yes, the work is:
1. Extract `am-py/python/agentmate/` as a standalone pure-Python package
2. Add missing AIS ops (7 methods in proxy.py)
3. Fix CLI integration to use `dekk apxm execute`
4. Set up as git submodule of APXM
5. Archive the Rust crates

---

## Verification

To validate that the lean Python frontend works end-to-end:
1. `cd external/agentmate && python examples/python/research_pipeline.py` — should print valid `.apxm` JSON
2. Pipe that JSON to `dekk apxm validate` — should pass
3. Run `dekk apxm execute <graph.apxm>` — should compile and execute
4. Run `python -m pytest crates/am-py/tests/` — all graph tests should pass
