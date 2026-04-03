# APXM Strategy Synthesis: Unified Orchestration and Execution

> **Note**: Written March 31, 2026. Phases 1–3 are now COMPLETE. See `docs/STATE-2026-04-03.md` for current status. CLI references to `apxm llm` below are stale; the current CLI uses `apxm backend`.

**Date**: April 1, 2026
**Scope**: APXM as the complete agent execution platform -- graph building, compilation, and execution in one system
**Source**: Strategy documents 01-08, architecture docs, implementation specs, patent alignment, codebase investigation

---

## Table of Contents

1. [Executive Summary](#1-executive-summary)
2. [The Core Insight: There Is No Orchestration Layer](#2-the-core-insight-there-is-no-orchestration-layer)
3. [What APXM Already Is](#3-what-apxm-already-is)
4. [The Complete Pipeline: Author to Execute](#4-the-complete-pipeline-author-to-execute)
5. [Graph Authoring: Three Native Paths](#5-graph-authoring-three-native-paths)
6. [The Agent Instruction Set (AIS)](#6-the-agent-instruction-set-ais)
7. [The Compilation Pipeline](#7-the-compilation-pipeline)
8. [The Runtime Engine](#8-the-runtime-engine)
9. [ACP: External Agents as a Capability](#9-acp-external-agents-as-a-capability)
10. [Integration with vLLM](#10-integration-with-vllm)
11. [Five Patent Techniques](#11-five-patent-techniques)
12. [Goal-Directed Optimization Targets](#12-goal-directed-optimization-targets)
13. [Host Integration: The Three-Axis Model](#13-host-integration-the-three-axis-model)
14. [Gap Analysis and Opportunities](#14-gap-analysis-and-opportunities)
15. [Implementation Roadmap](#15-implementation-roadmap)
16. [Performance Targets](#16-performance-targets)
17. [The LLVM Parallel](#17-the-llvm-parallel)

---

## 1. Executive Summary

APXM (Agent Programming eXecution Model) is a **complete agent execution platform** that unifies graph authoring, compilation, and runtime execution into a single system. There is no separate orchestration layer. The graph is built in APXM. It is compiled in APXM. It is executed in APXM.

The system provides:

- **Three authoring paths**: AIS DSL (`.ais`), ApxmGraph JSON, Python API
- **An MLIR compiler** with 7 optimization passes (6 more planned)
- **A parallel dataflow scheduler** with 10.37x measured speedup over sequential execution
- **39 AIS operations** spanning LLM calls, tool invocation, memory, control flow, multi-agent coordination, and error handling
- **Built-in ACP protocol support** with 17 agent profiles (Claude, Codex, Gemini, Copilot, and more) -- already implemented in the `apxm-acp` crate
- **A three-tier memory hierarchy** (STM/LTM/Episodic) with formal AAM state model

The remaining work focuses on deepening APXM's integration with vLLM (graph-aware inference scheduling, KV-cache pinning) and implementing five patent techniques (spaghetti-stack context, speculation, pipelining, memoization, stage-specific loading) along with goal-directed optimization targets (`--target tokens/parallel/latency/cost`).

```
One-sentence summary:

APXM is the compiler, orchestrator, and runtime -- graphs are authored
natively (AIS DSL, JSON, or Python), compiled through MLIR, and executed
on a parallel dataflow scheduler that dispatches to LLM backends, external
agents (via ACP), tools, and sandboxed processes, with no separate
orchestration layer needed.
```

---

## 2. The Core Insight: There Is No Orchestration Layer

### The Old Mental Model (Wrong)

```
  Orchestrator (ACPX)  ──▶  Compiler (APXM)  ──▶  Runtime (vLLM)
     separate                  separate              separate
```

This framing assumed you need a separate system to decide *what* to do (orchestration), then hand it to another system to *optimize* it (compilation), then hand it to yet another system to *run* it (execution). Each system blind to the others.

### The Actual Architecture

```
  ┌───────────────────────────────────────────────────────────────┐
  │                           APXM                                │
  │                                                               │
  │   Author ──▶ Compile ──▶ Execute ──▶ Observe                 │
  │   (AIS DSL)  (MLIR)     (Dataflow)  (Metrics)               │
  │   (JSON)     (Passes)   (Scheduler) (Episodic)              │
  │   (Python)   (.apxmobj) (Workers)   (Traces)                │
  │                                                               │
  │   Everything in one system.                                   │
  │   The graph IS the orchestration.                             │
  │   The compiler IS the optimizer.                              │
  │   The runtime IS the executor.                                │
  └───────────────────────────────────────────────────────────────┘
                    │                    │
                    ▼                    ▼
            ┌──────────────┐    ┌──────────────┐
            │    vLLM      │    │ ACP Agents   │
            │ (model serve)│    │ (Claude Code │
            │              │    │  Codex, etc.)│
            └──────────────┘    └──────────────┘
            External backends, not orchestration layers.
```

**Why there is no orchestration layer:**

APXM already has every capability that an orchestrator would provide:

| Orchestration Concern | APXM Implementation |
|----------------------|---------------------|
| Agent dispatch | `DELEGATE` op: full sub-DAG execution with scoped memory and AAM tracking |
| Inter-agent communication | `COMMUNICATE` op: local, HTTP, or broadcast fan-out |
| Agent spawning | `SPAWN_AGENT` op: runtime registration with capabilities and goals |
| Parallel execution | Work-stealing dataflow scheduler with 4-priority tiers |
| Conditional routing | `BRANCH`, `SWITCH` ops with full expression evaluation |
| Tool invocation | `INV` op with `CapabilitySystem`: schema validation, timeouts, interceptors |
| Sandbox execution | `EXC` op with `SandboxRegistry` for process isolation |
| Human-in-the-loop | `PAUSE` + `RESUME` ops with checkpoint serialization |
| Memory across steps | 3-tier: STM (microsecond), LTM (persistent), Episodic (audit trail) |
| Model routing | `OperationRoutes` + `BackendFallback` + per-op defaults |
| Optimization | MLIR compiler: FuseAskOps, CSE, DCE, critical path analysis |
| Observability | Metrics, events, episodic memory, `--emit-metrics` |
| External agent dispatch | `apxm-acp` crate: full ACP JSON-RPC 2.0 with 17 agent profiles |
| Compilation | `.ais` / `.json` --> MLIR --> `.apxmobj` binary artifacts |

---

## 3. What APXM Already Is

### The Crate Architecture (15 crates, 5 tiers)

```
                               ┌──────────┐
                               │ apxm-cli │  Tier 5: CLI
                               └────┬─────┘
                                    │
                               ┌────▼──────┐
                               │apxm-driver│  Tier 4: Orchestration
                               └────┬──────┘  (compile + configure + run)
                     ┌──────────────┼──────────────┐
                     │              │              │
               ┌─────▼──────┐ ┌────▼──────┐ ┌─────▼──────┐
               │apxm-compiler│ │apxm-      │ │apxm-       │  Tier 3
               │ (MLIR-based)│ │runtime    │ │backends    │
               └─────┬──────┘ └────┬──────┘ └─────┬──────┘
                     │             │              │
          ┌──────────┼─────────────┼──────────────┤
          │          │             │              │
    ┌─────▼────┐┌───▼────┐┌──────▼──┐┌──────────▼───┐
    │apxm-graph││apxm-   ││apxm-acp ││apxm-sandbox  │  Tier 2
    │          ││artifact││ (FULL   ││apxm-tools    │
    │          ││        ││  ACP    ││apxm-events   │
    │          ││        ││  IMPL)  ││apxm-creds    │
    └─────┬────┘└───┬────┘└─────────┘└──────────────┘
          │         │
     ┌────▼─────────▼────┐
     │     apxm-core     │  Tier 1: Foundation
     └────────┬──────────┘
              │
     ┌────────▼──────────┐
     │     apxm-ais      │  Tier 0: Single Source of Truth
     │  39 AIS operations │  (no dependencies)
     └───────────────────┘
```

### What `apxm-acp` Already Implements (Not Planned -- Done)

The `apxm-acp` crate is a **complete, production-grade ACP implementation**:

| Component | Status | Details |
|-----------|--------|---------|
| JSON-RPC 2.0 transport | Done | NDJson over stdio |
| Protocol negotiation | Done | `initialize` with version `2025-11-05` |
| Authentication | Done | `ACPX_AUTH_` env vars with priority resolution |
| Session lifecycle | Done | `session/new`, `session/prompt`, `session/cancel` |
| Session pooling | Done | `SessionPool` with DashMap, multi-turn support |
| Reverse requests | Done | File I/O (`fs/readTextFile`, `fs/writeTextFile`) |
| Terminal management | Done | `terminal/create`, `terminal/output`, `terminal/kill` |
| Permission system | Done | 3 modes: ApproveAll, ApproveReads, DenyAll |
| Agent registry | Done | **17 built-in profiles**: claude, codex, gemini, copilot, openclaw, pi, cursor, droid, kilocode, kimi, kiro, opencode, qoder, qwen, trae, iflow |
| Streaming responses | Done | `session/update` notifications with text accumulation |
| Token tracking | Done | `usage_update` notifications |
| Graceful shutdown | Done | SIGTERM --> SIGKILL fallback with configurable grace period |
| Driver integration | Done | Auto-registered as capability in `apxm-driver` |

**This means Phase 2 of the old 16-week plan (ACP Client) is already done.**

---

## 4. The Complete Pipeline: Author to Execute

```
  ┌─────────────────────────────────────────────────────────────┐
  │                                                             │
  │  1. AUTHOR                                                  │
  │                                                             │
  │     Option A: AIS DSL (.ais)                                │
  │     ┌─────────────────────────────────────────────┐        │
  │     │ agent PRReview {                            │        │
  │     │   capability gh_diff(pr: int) -> str;       │        │
  │     │   @entry flow main(pr: int) -> str {        │        │
  │     │     gh_diff(pr) -> diff                     │        │
  │     │     ask("Review: " + diff) -> review        │        │
  │     │     think("Verdict on: " + review) -> verdict│       │
  │     │     return verdict                          │        │
  │     │   }                                         │        │
  │     │ }                                           │        │
  │     └─────────────────────────────────────────────┘        │
  │                                                             │
  │     Option B: ApxmGraph JSON                                │
  │     ┌─────────────────────────────────────────────┐        │
  │     │ { "name": "pr-review",                      │        │
  │     │   "nodes": [                                │        │
  │     │     {"id":1, "op":"INV", ...},              │        │
  │     │     {"id":2, "op":"ASK", ...},              │        │
  │     │     {"id":3, "op":"THINK", ...}             │        │
  │     │   ],                                        │        │
  │     │   "edges": [{"from":1,"to":2}, ...] }       │        │
  │     └─────────────────────────────────────────────┘        │
  │                                                             │
  │     Option C: Python API (AgentMate)                        │
  │     ┌─────────────────────────────────────────────┐        │
  │     │ @ag.compile(opt_level=2)                    │        │
  │     │ def review(g, pr):                          │        │
  │     │     diff = g.invoke("gh_diff", pr)          │        │
  │     │     review = g.ask("review", f"Review {diff}")│      │
  │     │     return g.think("verdict", review)       │        │
  │     └─────────────────────────────────────────────┘        │
  │                                                             │
  └──────────────────────────┬──────────────────────────────────┘
                             │
                             ▼
  ┌──────────────────────────────────────────────────────────────┐
  │  2. VALIDATE + ANALYZE                                       │
  │                                                              │
  │  $ apxm validate review.ais        # Structure, types, DAG  │
  │  $ apxm analyze review.ais         # Parallelism, speedup   │
  │  $ apxm explain review.ais         # Human-readable summary │
  └──────────────────────────┬───────────────────────────────────┘
                             │
                             ▼
  ┌──────────────────────────────────────────────────────────────┐
  │  3. COMPILE (MLIR Pipeline)                                  │
  │                                                              │
  │  $ apxm compile review.ais -O2 --target latency             │
  │                                                              │
  │  Normalize --> Lower to AIS Dialect --> Optimize --> Emit    │
  │                                                              │
  │  Passes: FuseAskOps, CSE, DCE, Canonicalize,               │
  │          ContextBudget, ParallelismExtraction,              │
  │          SpeculationInsertion, PipelineInsertion,            │
  │          ModelDowngrade                                      │
  │                                                              │
  │  Output: review.apxmobj (52B header + BLAKE3 + bincode)    │
  └──────────────────────────┬───────────────────────────────────┘
                             │
                             ▼
  ┌──────────────────────────────────────────────────────────────┐
  │  4. EXECUTE (Dataflow Scheduler)                             │
  │                                                              │
  │  $ apxm run review.apxmobj 42                               │
  │  $ apxm execute review.ais 42   # compile + run in one step │
  │                                                              │
  │  ┌────────────────────────────────────────────┐             │
  │  │ Scheduler fires ops when inputs arrive:    │             │
  │  │                                            │             │
  │  │   INV(gh_diff) ──▶ ASK(review) ──▶ THINK  │             │
  │  │                                            │             │
  │  │ Dispatches to:                             │             │
  │  │   LLM backends (vLLM, OpenAI, Ollama)     │             │
  │  │   ACP agents (Claude Code, Codex, Gemini)  │             │
  │  │   Tools (bash, file I/O, MCP)              │             │
  │  │   Sandboxed processes (bwrap, docker)       │             │
  │  └────────────────────────────────────────────┘             │
  │                                                              │
  │  Output: result + metrics + episodic trace                  │
  └──────────────────────────────────────────────────────────────┘
```

---

## 5. Graph Authoring: Three Native Paths

### Path 1: AIS DSL (Production-Ready)

The recommended path for human-written workflows:

```ais
agent CodeReviewCouncil {
    memory {
        findings: LTM
    }

    capability search(query: str) -> str;

    flow review_code(code: str) -> str {
        // These three run IN PARALLEL (no data dependencies between them)
        ask(backend: "apxm", prompt: "Security review: " + code) -> security
        ask(backend: "apxm", prompt: "Quality review: " + code) -> quality
        ask(backend: "apxm", prompt: "Performance review: " + code) -> perf

        // Synthesis waits for all three (implicit WAIT_ALL from DAG)
        think(prompt: "Synthesize:\n" + security + quality + perf) -> synthesis

        store_fact(text: synthesis, tags: ["review"], source: "council")
        return synthesis
    }

    @entry flow main(pr_number: int) -> str {
        search("gh pr diff " + pr_number) -> diff
        review_code(diff) -> result
        return result
    }
}
```

Supported constructs: `agent` declarations, `@entry` flows, `capability` declarations, `memory` blocks, cross-agent flow calls, backend selection, token budgets, all 39 AIS operations.

### Path 2: ApxmGraph JSON (Stable API Contract)

For programmatic generation and external tools:

```json
{
  "name": "parallel-review",
  "parameters": [{"name": "question", "type_name": "str"}],
  "nodes": [
    {"id": 1, "name": "ask_codex", "op": "INV", "attributes": {
      "capability": "acp",
      "params_json": "{\"agent\": \"codex\", \"prompt\": \"Answer: {0}\"}",
      "timeout_ms": 300000
    }},
    {"id": 2, "name": "ask_claude", "op": "INV", "attributes": {
      "capability": "acp",
      "params_json": "{\"agent\": \"claude\", \"prompt\": \"Answer: {0}\"}",
      "timeout_ms": 300000
    }},
    {"id": 3, "name": "compare", "op": "ASK", "attributes": {
      "template_str": "Compare:\nCodex: {0}\nClaude: {1}\nWhich is better?"
    }}
  ],
  "edges": [
    {"from": 1, "to": 3, "dependency": "Data"},
    {"from": 2, "to": 3, "dependency": "Data"}
  ]
}
```

Nodes 1 and 2 have **no edge between them** -- the dataflow scheduler runs them **in parallel automatically**.

### Path 3: Python API (via AgentMate)

```python
@ag.compile(opt_level=2)
def parallel_review(g: ag.GraphRecorder, question: str):
    codex = g.invoke("ask_codex", capability="acp",
                     params={"agent": "codex", "prompt": f"Answer: {question}"})
    claude = g.invoke("ask_claude", capability="acp",
                      params={"agent": "claude", "prompt": f"Answer: {question}"})
    sync = g.wait_all("sync", codex, claude)
    result = g.ask("compare", "Compare:\nCodex: {0}\nClaude: {1}")
    sync >> result
    return result
```

All three paths converge on the same canonical `ApxmGraph` IR before compilation.

---

## 6. The Agent Instruction Set (AIS)

39 typed operations organized in 7 categories:

```
┌──────────────────────────────────────────────────────────────────┐
│                     AIS OPERATION MAP                            │
├──────────────────────────────────────────────────────────────────┤
│                                                                  │
│  COGNITIVE (LLM)              TOOLS              CONTROL FLOW   │
│  ┌──────────────────┐        ┌──────────┐       ┌────────────┐ │
│  │ ASK       (~1s)  │        │ INV      │       │ BRANCH     │ │
│  │ THINK     (~3s)  │        │ (invoke) │       │ SWITCH     │ │
│  │ REASON    (~10s) │        │ EXC      │       │ LOOP_START │ │
│  │ PLAN      (~3s)  │        │ (sandbox)│       │ LOOP_END   │ │
│  │ REFLECT   (~3s)  │        │ PRINT    │       │ RETURN     │ │
│  │ VERIFY    (~1s)  │        └──────────┘       │ FLOW_CALL  │ │
│  └──────────────────┘                           │ JUMP       │ │
│                              SYNCHRONIZATION    └────────────┘ │
│  COORDINATION               ┌──────────────┐                   │
│  ┌──────────────────┐       │ MERGE        │   MEMORY         │
│  │ UPDATE_GOAL      │       │ WAIT_ALL     │   ┌────────────┐ │
│  │ GUARD            │       │ FENCE        │   │ QMEM (read)│ │
│  │ CLAIM            │       └──────────────┘   │ UMEM (write│ │
│  │ PAUSE            │                          │ FENCE      │ │
│  │ RESUME           │       COMMUNICATION      └────────────┘ │
│  │ DELEGATE         │       ┌──────────────┐                   │
│  │ NEGOTIATE        │       │ TRY_CATCH    │   SELF-ORG       │
│  │ SPAWN_AGENT      │       │ ERR          │   ┌────────────┐ │
│  │ REGISTER_CAPAB.  │       │ COMMUNICATE  │   │ AUTONOMOUS │ │
│  └──────────────────┘       │ FLOW         │   │ NOP        │ │
│                              └──────────────┘   │ IDENTITY   │ │
│                                                 └────────────┘ │
│                                                                  │
│  LATENCY STRATIFICATION: While a 10s REASON runs, independent  │
│  ASK (~1s) operations execute concurrently. The scheduler       │
│  exploits this automatically from the graph topology.            │
└──────────────────────────────────────────────────────────────────┘
```

### Key Bridging Operations

| Operation | What It Does | Why Orchestrators Aren't Needed |
|-----------|-------------|--------------------------------|
| `DELEGATE` | Execute sub-DAG on target agent with scoped memory | APXM *is* the orchestrator -- sub-DAGs run on the same scheduler |
| `COMMUNICATE` | Local/HTTP/broadcast inter-agent messaging | No external message bus -- built into runtime |
| `SPAWN_AGENT` | Register new agent at runtime with capabilities | Dynamic agent creation without restarting |
| `FLOW_CALL` | Invoke named flow on another agent | Cross-agent dispatch via FlowRegistry |
| `INV(acp)` | Dispatch to external coding agent via ACP protocol | Claude/Codex/Gemini are just capabilities, not orchestrators |

---

## 7. The Compilation Pipeline

```
  Input (.ais / .json / Python)
            │
  ┌─────────▼──────────┐
  │ Stage 1: Normalize  │  Validate structure, IDs, DAG properties
  │                     │  49x faster error detection than runtime-only
  └─────────┬──────────┘
            │
  ┌─────────▼──────────┐
  │ Stage 2: Lower to   │  Create MLIR operations with type inference
  │ AIS Dialect          │  Custom verifiers for AIS invariants
  └─────────┬──────────┘
            │
  ┌─────────▼──────────────────────────────────────────┐
  │ Stage 3: Optimization Passes                        │
  │                                                     │
  │  IMPLEMENTED (7):                                   │
  │  Normalize, BuildPrompt, CapScheduling,            │
  │  FuseAskOps, CSE, DCE, Canonicalization            │
  │                                                     │
  │  PLANNED (6, via --target flag):                    │
  │  ContextBudget, DeadContextElim,                   │
  │  ParallelismExtraction, SpeculationInsertion,      │
  │  PipelineInsertion, ModelDowngrade                 │
  └─────────┬──────────────────────────────────────────┘
            │
  ┌─────────▼──────────┐
  │ Stage 4: Emit       │  .apxmobj binary artifact
  │ Artifact             │  52B header + BLAKE3 hash + bincode
  └─────────────────────┘
```

### Artifact Caching

The driver hashes each graph and caches compiled artifacts at `~/.cache/apxm/artifacts/{hash}`. Subsequent runs skip compilation entirely. This means `apxm execute graph.ais` has near-zero overhead on repeat invocations.

---

## 8. The Runtime Engine

### Dataflow Scheduler

```
  Tokens injected into graph
          │
          ▼
  ┌───────────────────────────────────────────────────┐
  │              Dataflow Scheduler                    │
  │                                                    │
  │  Token counting: each node tracks how many        │
  │  inputs it needs. When count reaches zero,        │
  │  the node is READY. O(1) readiness check.         │
  │                                                    │
  │  ┌─────────────┐    ┌────────────────┐            │
  │  │ DashMap      │───▶│ ReadySet       │            │
  │  │ (per-node    │    │ (lock-free)    │            │
  │  │  counters)   │    └───────┬────────┘            │
  │  └─────────────┘            │                      │
  │                    ┌────────▼────────┐             │
  │                    │ PriorityQueue   │             │
  │                    │ (4 tiers,       │             │
  │                    │  critical-path) │             │
  │                    └────────┬────────┘             │
  │                             │                      │
  │              ┌──────────────┼──────────────┐      │
  │              ▼              ▼              ▼      │
  │         ┌────────┐    ┌────────┐    ┌────────┐   │
  │         │Worker 1│    │Worker 2│    │Worker N│   │
  │         └────────┘    └────────┘    └────────┘   │
  │                                                    │
  │  Work-stealing, semaphore-based concurrency,      │
  │  backpressure, deadlock watchdog                  │
  └───────────────────────────────────────────────────┘
```

### Three-Tier Memory (AAM)

| Tier | Implementation | Access Time | Purpose |
|------|---------------|-------------|---------|
| **STM** | `DashMap<String, Value>` | microseconds | Per-execution scratch data |
| **LTM** | Embedded SQLite (WAL) | milliseconds | Persistent facts across sessions |
| **Episodic** | Append-only log | milliseconds | Timestamped event history |

### Agent Abstract Machine State

```
AAM = (B, G, C)

B (Beliefs):      HashMap<String, Value>        -- what the agent knows
G (Goals):        PriorityQueue<Goal>           -- what the agent wants
C (Capabilities): HashMap<String, CapRecord>    -- what the agent can do

Scoping policies for multi-agent:
  INHERIT  -- child sees parent + adds local state
  ISOLATE  -- clean slate, no parent visibility
  SNAPSHOT -- copy parent at creation, then diverge
  FILTER   -- selective access to specified keys
```

---

## 9. ACP: External Agents as a Capability

External coding agents (Claude Code, Codex, Gemini) are **not orchestrators**. They are capabilities that APXM dispatches to via the `INV(acp)` operation, just like any other tool.

### How It Works

```
  APXM Graph
       │
       │  node: { "op": "INV", "capability": "acp",
       │          "params_json": {"agent": "claude", "prompt": "..."} }
       │
       ▼
  ┌────────────────────────────────────────────────────────┐
  │  CapabilitySystem                                      │
  │                                                        │
  │  "acp" ──▶ AcpCapability                              │
  │            │                                           │
  │            ├─ Look up agent in registry (17 built-in)  │
  │            ├─ Get or create session from SessionPool   │
  │            ├─ Spawn subprocess if new session          │
  │            ├─ Send JSON-RPC: initialize                │
  │            ├─ Send JSON-RPC: session/new               │
  │            ├─ Send JSON-RPC: session/prompt            │
  │            ├─ Handle reverse requests:                 │
  │            │   ├─ fs/readTextFile                      │
  │            │   ├─ fs/writeTextFile                     │
  │            │   ├─ terminal/create                      │
  │            │   ├─ terminal/output                      │
  │            │   └─ requestPermission                    │
  │            ├─ Collect streaming response               │
  │            └─ Return result as Value                   │
  │                                                        │
  │  "bash" ──▶ ShellCapability                           │
  │  "read" ──▶ FileReadCapability                        │
  │  (etc.)                                                │
  └────────────────────────────────────────────────────────┘
```

### The Two-Tier LLM Model

```
                         APXM Graph
                             │
                ┌────────────┼────────────┐
                │            │            │
           ┌────▼────┐  ┌────▼────┐  ┌────▼────┐
           │  ASK    │  │  INV    │  │ DELEGATE│
           │  THINK  │  │  (acp)  │  │         │
           │  REASON │  │         │  │         │
           └────┬────┘  └────┬────┘  └────┬────┘
                │            │            │
         Direct LLM     ACP Agent    Sub-DAG
         (fast, cheap)  (powerful)   (registered)
                │            │            │
           ┌────▼────┐  ┌────▼────┐  ┌────▼────┐
           │  vLLM   │  │ Claude  │  │ Another │
           │  OpenAI │  │  Code   │  │  APXM   │
           │  Ollama │  │ Codex   │  │  graph  │
           │         │  │ Gemini  │  │         │
           └─────────┘  └─────────┘  └─────────┘
```

| Tier | Operation | Use When | Latency | Cost |
|------|-----------|----------|---------|------|
| **Direct LLM** | ASK, THINK, REASON | Analysis, reasoning, classification | 1-10s | Low |
| **ACP Agent** | INV(acp) | Multi-step coding: edit files, run tests, iterate | 30-300s | High |
| **Sub-DAG** | DELEGATE | Orchestrated multi-agent workflow | Varies | Varies |

The compiler can decide: a simple "review this diff" uses ASK (3s). A complex "fix this bug, run tests, iterate until green" uses INV(acp) with Claude Code (5 min).

---

## 10. Integration with vLLM

vLLM is an **external model serving backend**, not part of the orchestration. The opportunity is making it **graph-aware** so it can optimize inference for multi-stage pipelines.

### The Disconnection Today

vLLM treats each request independently. It doesn't know that:
- The output of Node A feeds into Node B (so keep KV-cache warm)
- Node C is on the critical path (so prioritize it)
- Node D's static context could be prefilled while Node C generates

### The Integration

```
  APXM Runtime                              vLLM
       │                                      │
       │  POST /v1/graphs/register            │
       │  { nodes, critical_path,             │
       │    prefillable_contexts }        ───▶ GraphAwareScheduler
       │                                      │
       │  POST /v1/completions                │
       │  X-APXM-Graph-Id: g1                │
       │  X-APXM-Node-Id: n3                 │
       │  X-APXM-Downstream: [n4, n5]        │
       │  X-APXM-Priority: critical       ───▶ Priority scheduling
       │                                      │  KV-cache pinning
       │                                      │  Eager prefill
       │                                      │
       │  ◀─── Streaming response             │
       │       (tokens pipelined to           │
       │        next node's prefill)          │
```

### Three Key Behaviors

1. **Result Pinning**: Keep KV-cache warm for nodes feeding downstream consumers
2. **Eager Prefill**: Begin prefilling next node's static context while current generates
3. **Priority Scheduling**: Critical-path nodes get higher GPU priority

---

## 11. Five Patent Techniques

**Patent**: "Spaghetti-Stack Context Management for LLM-Based AI Agents"
**Inventors**: APXM Contributors

```
          ┌──────────────────────────────────────────┐
          │  T1: SPAGHETTI-STACK CONTEXT              │
          │                                           │
          │  Context as cactus stack:                 │
          │  - Active branch loaded (STM)             │
          │  - Collapsed branches summarized (LTM)    │
          │  - Reference pointers (Episodic)          │
          │                                           │
          │  RESULT: 34% of flat context approach     │
          └────────┬──────────────┬───────────────────┘
                   │              │
       ┌───────────▼──┐     ┌────▼────────────┐
       │ T5: STAGE-   │     │ T3: TOKEN       │
       │ SPECIFIC     │     │ PIPELINING      │
       │ LOADING      │     │                 │
       │              │     │ Stream tokens   │
       │ context_     │     │ from Node A     │
       │ manifest per │     │ into Node B's   │
       │ node with    │     │ prefill while   │
       │ include/     │     │ A generates     │
       │ exclude/     │     │                 │
       │ max_tokens   │     │ 30-50% overlap  │
       └──────┬───────┘     └──────┬──────────┘
              │                    │
              │     ┌──────────────┘
              ▼     ▼
       ┌──────────────────┐
       │ T4: INPUT-KEYED  │────────────────────┐
       │ MEMOIZATION      │                    │
       │                  │             feeds  │
       │ Two-tier:        │                    ▼
       │ DashMap (us)     │          ┌──────────────────┐
       │ + SQLite (ms)    │          │ T2: SPECULATIVE  │
       │                  │          │ EXECUTION        │
       │ Only temp=0      │          │                  │
       └──────────────────┘          │ Start downstream │
                                     │ with memoized    │
                                     │ predictions.     │
                                     │ Commit or rollback│
                                     └──────────────────┘

  COMBINED:
  ┌────────────────────────────────────────────────────┐
  │ Context per stage:  100% --> 34%  (3x reduction)   │
  │ Memoized response:  ~1 microsecond                 │
  │ Pipeline overlap:   30-50%                         │
  │ Multi-agent:        up to 10.37x speedup           │
  └────────────────────────────────────────────────────┘
```

---

## 12. Goal-Directed Optimization Targets

```bash
apxm compile graph.ais -O2 --target latency     # minimize end-to-end time
apxm compile graph.ais -O2 --target cost         # minimize API costs
apxm compile graph.ais -O2 --target tokens       # minimize context usage
apxm compile graph.ais -O2 --target parallel     # maximize throughput
apxm compile graph.ais -O2                       # balanced (default)
```

### Target-to-Pass Mapping

| Target | Key Passes | Estimated Improvement |
|--------|-----------|----------------------|
| `tokens` | ContextBudget, DeadContextElim, FuseAskOps (aggressive) | 20-40% fewer tokens |
| `parallel` | ParallelismExtraction, SpeculationInsertion | 2-10x throughput |
| `latency` | PipelineInsertion, SpeculationInsertion, ParallelismExtraction | 30-50% latency cut |
| `cost` | ModelDowngrade, ContextBudget, aggressive memoization | 50-80% cost reduction |
| `balanced` | All passes at moderate settings | -- |

### Target-to-Patent Mapping

| Patent Technique | tokens | parallel | latency | cost | balanced |
|-----------------|--------|----------|---------|------|----------|
| T1: Spaghetti-stack | **Primary** | -- | -- | Enabled | -- |
| T2: Speculation | -- | **Primary** | **Primary** | Disabled | -- |
| T3: Token pipeline | -- | -- | **Primary** | -- | -- |
| T4: Memoization | Enabled | Enabled | Enabled | **Primary** | Enabled |
| T5: Stage loading | **Primary** | -- | Enabled | Enabled | -- |

### Visualization: Token Pipelining (`--target latency`)

```
                     Time ──────────────────────────▶

Without:  [=== Node A generate ===]  [=== Node B prefill ===][=== B gen ===]
                                      ^── waits for A

With:     [=== Node A generate ===]
               [== B prefill (static) ==]       <-- starts early
                    [== B prefill (stream) ==]   <-- appends A's output
                                          [=== B generate ===]
                                           ^-- starts sooner

          ~~~~~~~~~ 30-50% overlap ~~~~~~~~~
```

---

## 13. Host Integration: The Three-Axis Model

APXM ships zero platform-specific code. Hosts inject backends:

```
┌────────────────────────────────────────────────────────────────┐
│                    APXM Runtime (trait-based)                   │
│                                                                │
│  Axis 1: LLMBackend        Axis 2: Capabilities   Axis 3: Sandbox
│  (who thinks)               (what to do)           (how to isolate)
│                                                                │
│  trait LLMBackend {         trait CapabilityExec {  trait SandboxBk {
│    async generate();          async execute();       execute();
│    fn health_check();         fn describe();         validate();
│  }                          }                      }
└──────────┬──────────────────────┬──────────────────────┬───────┘
           │                      │                      │
    ┌──────▼──────┐        ┌──────▼──────┐        ┌──────▼──────┐
    │ Host LLMs   │        │ Host Tools  │        │ Host Sandbox│
    │ OpenAI      │        │ bash, files │        │ bubblewrap  │
    │ Anthropic   │        │ MCP tools   │        │ docker      │
    │ Ollama      │        │ ACP agents  │        │ seatbelt    │
    │ vLLM local  │        │ custom      │        │ custom      │
    └─────────────┘        └─────────────┘        └─────────────┘
```

Two reference integrations exist:

| Aspect | Codex (Rust-native) | Gemini (TypeScript + IPC) |
|--------|---------------------|---------------------------|
| LLM Backend | In-process `Arc<dyn Trait>` | HTTP+SSE sidecar |
| Sandbox | In-process (bwrap + seccomp) | Subprocess IPC |
| Capabilities | Partial (tools not yet dispatched) | Not yet connected |

---

## 14. Gap Analysis and Opportunities

### What's Done vs What's Needed

```
  DONE (Working Today):
  ┌────────────────────────────────────────────────────────┐
  │  39 AIS operations fully implemented                   │
  │  Parallel dataflow scheduler (10.37x measured speedup) │
  │  MLIR compiler with 7 optimization passes              │
  │  ACP protocol with 17 agent profiles (apxm-acp)       │
  │  3-tier memory (STM/LTM/Episodic)                     │
  │  AIS DSL + JSON authoring paths                        │
  │  Artifact caching (hash-based, zero recompile)         │
  │  FlowRegistry for cross-agent dispatch                 │
  │  CapabilitySystem with schema validation               │
  │  Codex + Gemini host integrations                      │
  │  AgentMate SDK (Rust + Python)                         │
  └────────────────────────────────────────────────────────┘

  REMAINING GAPS:
  ┌────────────────────────────────────────────────────────┐
  │                                                        │
  │  P0 (Critical):                                        │
  │  ┌──────────────────────────────────────────────────┐ │
  │  │ A1: Dynamic Model Routing        (2-3 weeks)    │ │
  │  │     Static model attr --> ModelRouter with       │ │
  │  │     health monitoring, circuit breaker           │ │
  │  └──────────────────────────────────────────────────┘ │
  │                                                        │
  │  P1 (Core):                                            │
  │  ┌──────────────────────────────────────────────────┐ │
  │  │ A2: Context Management           (4-5 weeks)    │ │
  │  │ A3: Memoization                  (2 weeks)      │ │
  │  │ V1: Graph-Aware vLLM             (4-5 weeks)    │ │
  │  │ V3: KV-Cache Pinning             (2-3 weeks)    │ │
  │  └──────────────────────────────────────────────────┘ │
  │                                                        │
  │  P2 (Optimization):                                    │
  │  ┌──────────────────────────────────────────────────┐ │
  │  │ A4: Speculative Execution        (3 weeks)      │ │
  │  │ A5: Token Pipelining             (4 weeks)      │ │
  │  │ V2: Incremental Prefill          (3-4 weeks)    │ │
  │  │ X1: Unified Observability        (3 weeks)      │ │
  │  │ 6 new compiler passes            (est. 1,230 LOC)│ │
  │  │ --target flag                    (est. 200 LOC) │ │
  │  └──────────────────────────────────────────────────┘ │
  │                                                        │
  │  P3 (Future):                                          │
  │  ┌──────────────────────────────────────────────────┐ │
  │  │ X2: Auto Graph Generation from NL (4-6 weeks)  │ │
  │  │ X3: PGO Feedback Loop             (3-4 weeks)  │ │
  │  └──────────────────────────────────────────────────┘ │
  └────────────────────────────────────────────────────────┘
```

---

## 15. Implementation Roadmap

With ACP already done, the plan simplifies. The old Phase 2 ("ACP Client") is complete. The roadmap becomes:

```
Week  1──2──3──4──5──6──7──8──9──10──11──12──13──14──15
      │        │              │                         │
      ▼        ▼              ▼                         ▼

  PHASE 1       PHASE 2           PHASE 3              PHASE 4
  Model Router  Context + Memo    Graph-Aware vLLM     Patent + Targets
  (Wks 1-3)     (Wks 4-6)        (Wks 7-11)           (Wks 12-15)

  ┌──────────┐  ┌──────────┐     ┌──────────────┐    ┌──────────────┐
  │ModelRouter│  │ContextStk│     │GraphAwareBknd│    │Speculation   │
  │HealthMon.│  │MemoCache │     │GraphMetadata │    │Pipelining    │
  │CircuitBrk│  │(DashMap+ │     │KV-CachePin  │    │              │
  │models.toml│ │ SQLite)  │     │EagerPrefill  │    │6 new compiler│
  │CLI: models│ │Manifests │     │PrioritySched │    │passes        │
  │           │  │          │     │Benchmarks    │    │--target flag │
  └──────────┘  └──────────┘     └──────────────┘    └──────────────┘

  Dependencies:
  Phase 1 & 2: Can proceed in parallel
  Phase 3: Depends on Phase 1 (model router backend interface)
  Phase 4: Depends on Phase 3 (pipelining needs vLLM streaming)
```

**Saved time**: ~3 weeks from the old plan (ACP is already done). Total: 15 weeks instead of 16. The saved time can be allocated to Phase 4's optimization targets.

---

## 16. Performance Targets

### Measured Today

| Workload | Sequential | APXM Parallel | Speedup |
|----------|-----------|---------------|---------|
| 3-agent research | 12.4s | 1.2s | **10.37x** |
| 2-agent code review | 8.1s | 2.3s | **3.52x** |
| 5-agent data pipeline | 31.0s | 5.8s | **5.34x** |

### Projected (After Phases 1-4)

| Metric | Current | Target |
|--------|---------|--------|
| Context per stage | 100% (flat) | **34%** (spaghetti stack) |
| Memoized response | N/A | **~1 microsecond** |
| Model failover | Manual / none | **Automatic** (circuit breaker) |
| Pipeline overlap | 0% | **30-50%** (token pipelining) |
| Compiler passes | 7 | **13** (+6 new goal-directed) |
| Optimization targets | 1 (level) | **5** (tokens/parallel/latency/cost/balanced) |

---

## 17. The LLVM Parallel

| Property | LLVM | APXM |
|----------|------|------|
| **Clean IR** | LLVM IR (typed, SSA) | AIS (39 ops, typed edges, dataflow tokens) |
| **Modular Passes** | ~200+ passes | 7 implemented + 6 planned |
| **Permissive License** | BSD/Apache 2.0 | MIT |
| **Solves 80% Problem** | Architecture-independent optimization | Agent-infrastructure shared across all frameworks |
| **Native Frontend** | Clang | AgentMate |
| **Proof of Concept** | LLVM-GCC (proved IR works) | Codex-on-APXM (proves AIS works) |

### The Compounding Advantage

```
  Better FuseAskOps     --> fewer LLM calls for ALL agents
  Better prompt caching  --> lower token cost for ALL agents
  Better scheduler       --> more parallelism for ALL agents
  Better memoization     --> fewer repeated calls for ALL agents
  Better model routing   --> cheaper execution for ALL agents
```

No single-agent project justifies building 13+ optimization passes. APXM builds them once; all agents benefit.

### The Adoption Path

1. **Codex-on-APXM**: Prove AIS can represent production coding agents
2. **AgentMate**: Purpose-built SDK (the "Clang" of APXM)
3. **Second adopter**: Another framework targets APXM (proves generality)
4. **Optimization moat**: Community-contributed passes benefit everyone
5. **Ecosystem**: Debugging, profiling, verification all built on AIS

---

## Appendix A: CLI Quick Reference

```bash
# Author + validate
apxm validate review.ais
apxm analyze review.ais
apxm explain review.ais

# Compile
apxm compile review.ais -o review.apxmobj -O2
apxm compile review.ais -O2 --target latency

# Execute
apxm execute review.ais 42                  # compile + run
apxm run review.apxmobj 42                  # pre-compiled

# Models + Agents
apxm models list
apxm models health
apxm llm add my-openai --provider openai --api-key sk-...

# Debugging
apxm execute review.ais --trace debug --emit-metrics metrics.json

# Templates
apxm template list
apxm template show fan-out --json

# Graph composition
apxm task merge step-a.json step-b.json -o combined.json

# Targets
apxm targets list
apxm targets show latency
```

## Appendix B: Configuration

**LLM Backends** (`~/.apxm/config.toml`):
```toml
[chat]
default_backend = "my-openai"

[[llm_backends]]
name = "my-openai"
provider = "openai"
# api_key resolved from environment or credential store

[routing]
# Per-operation model routing
[routing.operation_routes]
ASK = "fast-model"
THINK = "reasoning-model"
REASON = "best-model"
```

**Agent Profiles** (built into `apxm-acp`, overridable):
```toml
# ~/.apxm/agents.toml (user overrides)
[[agents]]
profile = "claude"
command = "npx"
args = ["-y", "@agentclientprotocol/claude-agent-acp@^0.24.2"]
permission_mode = "approve-reads"
timeout_ms = 300000
```

## Appendix C: Why There Is No Separate Orchestration Layer

| What an orchestrator does | How APXM does it natively |
|---------------------------|---------------------------|
| Define workflow structure | AIS DSL / JSON / Python graph |
| Route to different agents | `INV(acp)` with agent selection, or `DELEGATE` to registered flows |
| Execute steps in order | Dataflow scheduler fires ops when inputs ready |
| Execute steps in parallel | Automatic from graph topology (no edges = parallel) |
| Handle errors | `TRY_CATCH` with recovery subgraphs |
| Manage state across steps | AAM (B,G,C) with 3-tier memory |
| Human-in-the-loop | `PAUSE` + `RESUME` with checkpoint serialization |
| Conditional branching | `BRANCH`, `SWITCH` ops |
| Optimize execution | MLIR compiler with 13 passes |
| Observe execution | Metrics, episodic memory, tracing |
| Dispatch to external agents | `apxm-acp` crate: full ACP JSON-RPC 2.0 protocol |

**The graph is the orchestration. The compiler is the optimizer. The runtime is the executor. One system.**
