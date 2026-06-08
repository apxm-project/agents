# apxm-cli

Command-line interface for the APXM graph compiler and runtime toolchain.

## Overview

`apxm-cli` provides the `apxm` binary with subcommands for compiling, executing, validating, and inspecting APXM graphs. Invoke it through `dekk apxm ...` so the managed environment, toolchain, and helper scripts stay consistent. The crate wraps `apxm-driver` for compile/run operations and uses dekk for environment detection.

## Module Structure

| Module | Description |
|--------|-------------|
| `commands/cli` | Clap CLI definition (`Cli`, `Commands` enum) |
| `commands/implementations` | Command handler functions |
| `commands/mod` | Command dispatch and shared helpers |
| `frontend/codegen` | `codegen frontend` -- generates Python frontend code from the shared graph contract |
| `frontend/codegen_ts` | `codegen typescript` -- generates TypeScript types from the shared graph contract |
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
| `process` | List or stop APXM compile/run/execute/workflow jobs |
| `doctor` | Diagnose MLIR/LLVM/conda dependencies |
| `backend` | Add/list/remove/test LLM backends |
| `agent` | Add/list/remove/test agent profiles |
| `tool` | Register/list/remove external tools |
| `team` | Manage multi-agent teams |
| `ops` | List/show AIS operations |
| `template` | List/show graph templates |
| `codegen` | Generate frontend (Python) and TypeScript code from AIS definitions |
| `session` | Session management |
| `workflow` | `.apxmw` workflow-file management |
| `goal` | Start, follow, inspect, or cancel a bounded APXM goal orchestration run |
| `chat` | Interactive REPL over a running `apxm-server` |
| `rollout` | Inspect, replay, and archive rollout transcripts |
| `cache` | Cache management |

## Complex Work Paths

Use `goal` when an agent or human wants APXM to create one bounded worker DAG,
start it through the server, and wait on workflow events:

```bash
dekk apxm goal "Investigate and implement the scoped change" \
  --workspace git_worktree \
  --repo-root /path/to/repo \
  --worker research:"Inspect relevant code":worker-a \
  --worker implement:"Make the patch":worker-b \
  --worker verify:"Run checks":worker-c \
  --depends implement=research \
  --depends verify=implement
```

`goal` calls the server-owned orchestration path and follows
`apxm_workflow_events/status` unless `--no-follow` is set. Use `--status`,
`--events`, or `--cancel` with the returned execution id to inspect or stop a
run later.

Use `workflow run` for checked-in `.apxmw` files. `workflow execute` is a
visible alias for `workflow run`, so raw `apxm` and `dekk apxm workflow
execute` both work.

## Key Exports

- `Cli` -- top-level Clap parser
- `Commands` -- enum of all subcommands

## Dependencies

| Crate | Purpose |
|-------|---------|
| apxm-driver | Compilation and execution orchestration (behind `driver` feature) |
| apxm-core | Shared graph contract, types, and error codes |
| apxm-compiler | `AirModule` for validation and codegen |
