# apxm-driver

Orchestration layer bridging compiler and runtime.

## Overview

`apxm-driver` is the integration layer that loads configuration, links compiler output to the runtime, configures LLM backends, manages session output, resolves skills, and provides the `Linker` API for end-to-end execution.

## Module Structure

| Module | Description |
|--------|-------------|
| `config/` | `ApXmConfig`, `ChatConfig` -- TOML-based configuration (`~/.apxm/config.toml`) |
| `linker/` | `Linker`, `LinkerConfig`, `LinkResult` -- compile + execute orchestration |
| `compiler/` | Compiler wrapper for `AirModule` parsing and MLIR lowering |
| `runtime/mod` | `RuntimeExecutor` -- runtime with LLM backends configured |
| `runtime/llm` | LLM registry setup from config |
| `runtime/inner_plan` | `CompilerInnerPlanLinker` for dynamic sub-graph compilation |
| `runtime/agents` | Agent spawning and ACP integration |
| `runtime/capabilities` | Capability registration from config |
| `runtime/sandbox` | Sandbox backend selection |
| `runtime/sandbox_linux` | Linux-specific sandbox implementation |
| `cache` | Compilation and artifact caching |
| `context_assembler` | Context window assembly for multi-node execution |
| `session_output` | Session directory writer (`manifest.json`, `trace.ndjson`, per-node workspaces) |
| `skill_resolver` | Skill file discovery and injection into agent workspaces |
| `error` | `DriverError` type |

## Key Exports

- `Linker` / `LinkerConfig` / `LinkResult` -- end-to-end compile + execute
- `ApXmConfig` / `ChatConfig` / `ToolConfig` -- configuration types
- `RuntimeExecutor` -- runtime with backends wired up
- `DriverError` -- error type for driver operations

## Dependencies

| Crate | Purpose |
|-------|---------|
| apxm-compiler | Compilation pipeline |
| apxm-runtime | Execution engine |
| apxm-backends | LLM/storage providers |
| apxm-artifact | Artifact handling |
| apxm-core | Shared types and errors |
