# apxm-core

Shared graph contract, types, error definitions, event system, and constants used across APXM crates.

## Overview

`apxm-core` provides the downstream contract surface for the APXM toolchain: execution graph primitives, shared operation and pass metadata, structured errors with codes and suggestions, an open event system, graph constants, and backend-neutral model capability types.

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
| `types/` (root) | `GoalTree`, typed graph/runtime contracts, backend-neutral capability types |
| `error/` | `RuntimeError`, `CompilerError`, `CompileError`, `CliError`, `SecurityError` |
| `error/codes` | 50+ structured error codes (E001-E999) |
| `error/builder` | `ErrorBuilder` pattern with source locations and suggestions |
| `events/` | `ApxmEvent` envelope, the `EventPayload` trait, `EventEmitter` trait |
| `events/builder` | Fluent event construction |
| `events/emitter` | `EventEmitter` trait for pluggable event sinks |
| `events/registry` | Extension registry for non-core payload decoding |
| `constants` | graph attributes, JSON-RPC, protocol, and diagnostic constants |
| `agent_profile` | `AgentProfile` type for agent configuration |
| `handler_manifest` | `HandlerManifest`, `HandlerLanguage` (Python + TypeScript) |
| `plan` | `Plan`, `PlanStep`, `InnerPlanPayload` for dynamic sub-workflows |
| `paths` | Path utilities for `~/.apxm/` directory layout |
| `env` | `APXM_HOME` / `APXM_STATE_HOME` resolution |
| `logging` | Logging macros and configuration |
| `observability` | Observability surfaces |
| `utils/build` | Build-script helpers for native toolchain detection |

## Event System

`EventPayload` is an **open trait**, not a closed enum. Any type can implement it
via the `impl_event_payload!` macro, and consumers downcast through
`downcast_ref`. Core payloads decode first; `events/registry` supplies an
explicit extension registry for the rest.

The named kinds are generated, not hand-listed:
`SCHEMA_EVENT_KIND_REGISTRY` in `events/generated_event_kind_registry.rs` holds
60 entries, each carrying its category, whether it is terminal, and its terminal
sense. Read that file for the live set — do not maintain a count here.

### Consumers

| Consumer | Format | How |
|----------|--------|-----|
| Session inspection | Session directory | `apxm session list` / `inspect` / `diff` |
| Rollout replay | Monospace tree | `apxm replay <thread-id>` |
| Rollout archive | `.tar.gz` (JSONL + blobs) | `apxm archive <thread-id>` |

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
