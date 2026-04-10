# apxm-cli

Command-line interface for the APXM agent workflow toolchain.

## Overview

`apxm-cli` provides the `apxm` binary with subcommands for compiling, executing, validating, and managing agent workflows. It wraps `apxm-driver` for compile/run operations and uses dekk for environment detection.

## Module Structure

| Module | Description |
|--------|-------------|
| `commands/cli` | Clap CLI definition (`Cli`, `Commands` enum) |
| `commands/implementations` | Command handler functions |
| `commands/mod` | Command dispatch and shared helpers |
| `frontend/codegen` | `codegen frontend` -- generates Python frontend code from AIS definitions |
| `frontend/codegen_ts` | `codegen typescript` -- generates TypeScript types from AIS definitions |
| `frontend/registry` | Frontend code generation registry |
| `frontend/mod` | Frontend subcommand dispatch |

## Commands

| Command | Description |
|---------|-------------|
| `init` | Scaffold project directories and `apxm.toml` |
| `compile` | Compile `.air` to `.apxmobj` artifact |
| `execute` | Compile + run in one step |
| `run` | Execute a pre-compiled artifact |
| `decompile` | Reverse-map artifact back to graph |
| `validate` | Check graph against AIS contract |
| `analyze` | Parallelism, critical path, speedup estimate |
| `explain` | Human-readable summary of a graph |
| `replay` | Replay a session trace as timeline |
| `doctor` | Diagnose MLIR/LLVM/conda dependencies |
| `activate` | Print shell exports for MLIR/LLVM env setup |
| `install` | Install/update conda env from `environment.yaml` |
| `backend` | Add/list/remove/test LLM backends |
| `agent` | Add/list/remove/test agent profiles |
| `tool` | Register/list/remove external tools |
| `team` | Manage multi-agent teams |
| `ops` | List/show AIS operations |
| `template` | List/show workflow templates |
| `task` | Merge graph fragments |
| `codegen` | Generate frontend (Python) and TypeScript code from AIS definitions |
| `session` | Session management |
| `workflow` | Workflow management |
| `cache` | Cache management |
| `gui` | Launch the web visualization server |

## Key Exports

- `Cli` -- top-level Clap parser
- `Commands` -- enum of all subcommands

## Dependencies

| Crate | Purpose |
|-------|---------|
| apxm-driver | Compilation and execution orchestration (behind `driver` feature) |
| apxm-core | Shared types, error codes |
| apxm-ais | Operation metadata for `ops` commands |
| apxm-compiler | `AirModule` for validation and codegen |
