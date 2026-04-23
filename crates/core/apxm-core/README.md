# apxm-core

Shared graph contract, types, error definitions, event system, and constants used across APXM crates.

## Overview

`apxm-core` provides the downstream contract surface for the APXM toolchain: execution graph primitives, shared operation and pass metadata, structured errors with codes and suggestions, a 33-variant event system, graph constants, and provider/model specifications.

## Module Structure

| Module | Description |
|--------|-------------|
| `types/execution/` | `Agent`, `AgentFlow`, `ExecutionDag`, `Node`, `Edge`, `Status`, `Task` |
| `types/values/` | `Value`, `Token`, `Number` runtime value types |
| `types/compiler/` | `PassInfo`, `PassCategory`, `OptimizationLevel`, `CodegenOptions`, `Stages` |
| `types/operations/` | `AISOperationType`, `OperationCategory`, generated operation metadata |
| `types/session/` | Session management types |
| `types/identifiers/` | Typed IDs (`NodeId`, `TokenId`) |
| `types/intents/` | Intent types for goal-directed execution |
| `types/models/` | `ModelInfo`, `ModelCapabilities`, `ModelResponse` |
| `types/` (root) | `ProviderSpec`, `ProviderProtocol`, `BackendConfig`, `GoalTree` |
| `error/` | `RuntimeError`, `CompilerError`, `CompileError`, `CliError`, `SecurityError` |
| `error/codes` | 50+ structured error codes (E001-E999) |
| `error/builder` | `ErrorBuilder` pattern with source locations and suggestions |
| `events/` | `ApxmEvent` envelope, 33 `EventPayload` variants, `EventEmitter` trait |
| `events/builder` | Fluent event construction |
| `events/emitter` | `EventEmitter` trait for pluggable event sinks |
| `constants` | graph attributes, JSON-RPC, protocol, and diagnostic constants |
| `agent_profile` | `AgentProfile` type for agent configuration |
| `model_profiles` | Model capability profiles and token limits |
| `plan` | `Plan`, `PlanStep`, `InnerPlanPayload` for dynamic sub-graphs |
| `paths` | Path utilities for `~/.apxm/` directory layout |
| `logging` | Logging macros and configuration |
| `utils/build` | Build-script helpers for native toolchain detection |

## Event System (33 payloads)

Three layers of events flow through a single `ApxmEvent` envelope (`EventMeta` + `EventPayload`):

### LLM Layer (9)
Token, Thought, ToolCall, LlmDone, Usage, Retry, Warning, Citation, ProviderEvent

### Runtime Layer (16)
OperationStart/End, ToolStart/End, PlanCreated, PlanStepStarted/Completed, MemoryRead/Write, CheckpointSaved/Restored, SchedulerDecision, GpuUtilization, TokenUsage, MemoizationHit, Error

### Session Layer (8)
ContextCompacted, ModelRerouted, Cancelled, LoopDetected, ContextWindowWarning, SessionStart, SessionEnd, TurnBoundary

### Consumers

| Consumer | Format | How |
|----------|--------|-----|
| Session recording | NDJSON (`trace.ndjson`) | Written during `--emit-session` |
| GUI live view | SSE stream | `/live/sessions/:id` endpoint |
| CLI replay | Timeline rendering | `apxm replay <session-dir>` |
| Metrics collection | Aggregated stats | `--emit-metrics` flag |

## Key Exports

- `ExecutionDag` / `Node` / `Edge` -- execution graph primitives
- `Value` / `Token` / `Number` -- runtime value representations
- `RuntimeError` / `CompilerError` / `CompileError` -- structured errors
- `ApxmEvent` / `EventPayload` / `EventEmitter` -- event system
- `AISOperationType` / `DependencyType` -- operation and edge enums
- `Plan` / `PlanStep` / `InnerPlanPayload` -- dynamic planning types

## Dependencies

| Crate | Purpose |
|-------|---------|
| apxm-ais | Build-time authoring/codegen source only |
