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
| `compile-service-canonical` | Compile a canonical source package to `apxm.air.v1` JSON |
| `execute-canonical` | Execute canonical `apxm.air.v1` JSON through the canonical runtime |
| `doctor` | Diagnose MLIR/LLVM/conda dependencies |
| `backend` | Manage registered inference backend endpoints |
| `tool` | Register/list/remove external tools |
| `team` | Manage multi-agent teams |
| `agent` | Scaffold, sync, lint, build, and install agents |
| `org` | Scaffold, lint, and install organization packages |
| `integration` | Scaffold, lint, and install integration packages |
| `ops` | List/show AIS operations |
| `validate` | Check AIR against AIS contract |
| `analyze` | Parallelism, critical path, speedup estimate |
| `template` | List/show workflow templates |
| `explain` | Human-readable summary of a workflow |
| `codegen` | Generate frontend (Python) and TypeScript code from AIS definitions |
| `canonical-air` | Lower `apxm.frontend-graph.v1` JSON through the native compiler bridge |
| `session` | Session management |
| `process` | List or stop canonical APXM job processes (`canonical-air`, `compile-service-canonical`, `execute-canonical`) |
| `cache` | Cache management |
| `tokenize` | Report standalone token-accounting availability for text |
| `watch` | Stream a run's dispatch tree from `apxm-server` |
| `rollout` | Inspect, replay, and archive rollout transcripts |
| `chat` | Interactive REPL over a running `apxm-server` |

## Complex Work Paths

Use `chat` for a conversational loop over `apxm-server`. Pass `--agent <id>` to
open a server-backed session (`POST /v1/agents/{id}/sessions`) and pipe stdin
turns to it. The server address must be given explicitly via `--server <URL>`
or `APXM_SERVER_BASE` (there is no hardcoded default):

```bash
dekk agents chat --agent my-agent --server "$APXM_SERVER_BASE"
```

## Key Exports

- `Cli` -- top-level Clap parser
- `Commands` -- enum of all subcommands

## Dependencies

| Crate | Purpose |
|-------|---------|
| apxm-core | Shared workflow contract, types, and error codes |
| apxm-program | Canonical FrontendGraph, AIR, and executable artifact types |
| apxm-execution | Canonical local execution driver |
