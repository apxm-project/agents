# apxm-runtime

Dataflow execution engine for compiled APXM programs.

## Overview

`apxm-runtime` executes compiled AIS artifacts using a work-stealing dataflow scheduler. It provides a three-tier AAM memory system, a capability registry for tool integration, a model router with circuit breakers, and 37 operation handler modules dispatching 39 of the 41 AIS operations (ASK/THINK/REASON share one `llm` handler; AGENT and YIELD are pseudo-ops with no handler and resolve to a no-op in the dispatcher).

## Module Structure

| Module | Description |
|--------|-------------|
| `runtime` | `Runtime` and `RuntimeConfig` top-level entry points |
| `scheduler/` | Dataflow scheduler with work-stealing, lane queues, concurrency control |
| `executor/` | DAG execution engine, operation dispatch, memoization, cancellation |
| `executor/handlers/` | 37 operation handler modules (dispatching 39 of the 41 AIS operations; ASK/THINK/REASON share one `llm` handler) |
| `executor/pipeline` | Execution pipeline stages |
| `executor/token_accounting` | Token budget tracking per node |
| `executor/dag_splicer` | Dynamic DAG splicing for inner plans |
| `aam/` | Agent Abstract Machine (beliefs, goals, capabilities, effects, scope) |
| `memory/` | Three-tier memory: STM, LTM, Episodic, facts |
| `capability/` | `CapabilitySystem`, `FlowRegistry`, interceptors, built-in tools |
| `capability/builtins/` | Built-in capabilities: bash, read, write, web_search |
| `model_router/` | Model routing with health monitoring, circuit breakers, rate limiting |
| `context_stack/` | Context assembly, frames, budget tracking, eviction policy |
| `sandbox/` | Sandbox backend interface, security manifests, process isolation |
| `agent_pool` | `AgentPool` for reusing agent sessions |
| `process` / `process_table` | Process model: `AgentProcess`, `ProcessTable`, spawner/prompter |
| `thread` | `AgentThread` and thread state management |
| `team/` | Team registry for multi-agent coordination |
| `workflow/` | Workflow definitions, runner, templates, topological ordering |
| `workspace/` | Per-node workspace directory management |
| `observability/` | `MetricsCollector`, `SchedulerMetrics` |

## Key Exports

- `Runtime` / `RuntimeConfig` -- top-level runtime entry point
- `DataflowScheduler` / `SchedulerConfig` -- work-stealing dataflow scheduler
- `ExecutorEngine` / `ExecutionContext` -- DAG execution engine
- `Aam` -- Agent Abstract Machine state
- `MemorySystem` / `MemoryConfig` -- STM/LTM/Episodic memory
- `CapabilitySystem` / `FlowRegistry` -- tool and flow registration
- `ModelRouter` / `ModelRouterConfig` -- model routing with circuit breakers
- `ContextStack` / `ContextAssembly` -- context window management
- `AgentPool` / `ProcessTable` -- agent lifecycle management
- `SandboxBackend` / `SandboxRegistry` -- sandboxed execution interface

## Memory System

The runtime provides a 3-tier memory system accessed by `QMEM` (read) and `UMEM` (write) operations:

| Tier | Lifetime | Implementation |
|------|----------|----------------|
| **STM** (Short-Term) | Within execution | In-memory HashMap |
| **LTM** (Long-Term) | Persistent | Storage backend (SQLite/Redb) |
| **Episodic** | Session traces | Session NDJSON files |

## Dependencies

| Crate | Purpose |
|-------|---------|
| apxm-core | Execution graph types, values, errors, events |
| apxm-backends | LLM providers, storage backends |
| apxm-artifact | Artifact deserialization |
