# apxm-cli

Command-line interface for the APXM workflow compiler and runtime toolchain.

## Overview

`apxm-cli` provides the `apxm` binary with subcommands for canonical authoring, execution, validation, and APXM operations. Invoke it through `dekk agents ...` so the managed environment, toolchain, and helper scripts stay consistent.

## Module Structure

| Module | Description |
|--------|-------------|
| `commands/cli` | Clap CLI definition (`Cli`, `Commands` enum) |
| `commands/implementations` | Command handler functions |
| `commands/mod` | Command dispatch and shared helpers |
| `frontend/codegen` | `codegen frontend` -- generates Python frontend code from the shared workflow contract |
| `frontend/codegen_ts` | `codegen typescript` -- generates TypeScript types from the shared workflow contract |
| `frontend/registry` | Frontend code generation registry |
| `frontend/mod` | Frontend subcommand dispatch |

## Commands

| Command | Description |
|---------|-------------|
| `init` | Scaffold project directories and `apxm.toml` |
| `canonical-air` | Lower `apxm.frontend-graph.v1` JSON through the native compiler bridge |
| `compile-service-canonical` | Compile a canonical source package to `apxm.air.v1` JSON |
| `execute-canonical` | Execute canonical `apxm.air.v1` JSON through the canonical runtime |
| `decompile` | Reverse-map artifact back to AIR |
| `validate` | Check AIR against AIS contract |
| `analyze` | Parallelism, critical path, speedup estimate |
| `explain` | Human-readable summary of a workflow |
| `replay` | Replay a session trace as timeline |
| `process` | List or stop canonical APXM job processes (`canonical-air`, `compile-service-canonical`, `execute-canonical`) |
| `doctor` | Diagnose MLIR/LLVM/conda dependencies |
| `backend` | Add/list/remove/test LLM backends |
| `agent` | Add/list/remove/test agent profiles |
| `tool` | Register/list/remove external tools |
| `team` | Manage multi-agent teams |
| `ops` | List/show AIS operations |
| `template` | List/show workflow templates |
| `codegen` | Generate frontend (Python) and TypeScript code from AIS definitions |
| `session` | Session management |
| `workflow` | `.apxmw` workflow-file management |
| `goal` | Start, follow, inspect, or cancel a bounded APXM goal run |
| `chat` | Interactive REPL over a running `apxm-server` |
| `rollout` | Inspect, replay, and archive rollout transcripts |
| `cache` | Cache management |

## Complex Work Paths

Use `chat` for a conversational loop over `apxm-server`. By default it runs a
direct server-side ASK turn. Pass `--agent claude` to make each turn spawn and
communicate with an ACP Claude profile instead; `--agent-model` requests a
specific model when the selected ACP profile supports model control:

```bash
dekk agents chat --agent claude --agent-model claude-3-5-haiku-latest
```

Use `workflow run` for checked-in `.apxmw` files.

## Key Exports

- `Cli` -- top-level Clap parser
- `Commands` -- enum of all subcommands

## Dependencies

| Crate | Purpose |
|-------|---------|
| apxm-core | Shared workflow contract, types, and error codes |
| apxm-program | Canonical FrontendGraph, AIR, and executable artifact types |
| apxm-execution | Canonical local execution driver |
