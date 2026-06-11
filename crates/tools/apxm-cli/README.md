# apxm-cli

Command-line interface for the APXM workflow compiler and runtime toolchain.

## Overview

`apxm-cli` provides the `apxm` binary with subcommands for compiling, executing, validating, and inspecting APXM AIR workflows. Invoke it through `dekk apxm ...` so the managed environment, toolchain, and helper scripts stay consistent. The crate wraps `apxm-driver` for compile/run operations and uses dekk for environment detection.

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
| `compile` | Compile `.air` to `.apxmobj` artifact |
| `execute` | Compile + run in one step |
| `run` | Execute a pre-compiled artifact |
| `decompile` | Reverse-map artifact back to AIR |
| `validate` | Check AIR against AIS contract |
| `analyze` | Parallelism, critical path, speedup estimate |
| `explain` | Human-readable summary of a workflow |
| `replay` | Replay a session trace as timeline |
| `process` | List or stop APXM compile/run/execute/workflow jobs |
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

Use `goal` when an agent or human wants APXM to own a task, plan bounded worker
passes, and supervise the run through the server:

```bash
dekk apxm goal "Investigate and implement the scoped change" \
  --workspace git_worktree \
  --repo-root /path/to/repo
```

`goal` calls `goal_start` once and follows the goal event stream by the returned
`goal_id` unless `--no-follow` is set. By default the CLI omits `workers`, so
the server asks the APXM planner route for a bounded worker workflow, validates it,
and requests server-side ACP profile auto-selection. Use repeatable `--worker` plus `--depends` only
when the worker workflow must be pinned manually. Use `--status`, `--events`, or `--cancel`
with the returned `goal_id` to inspect or stop a run later. Status responses
expose the task ledger as `task.description`, `task.plan`, and `task.planning`.

Use `chat` for a conversational loop over `apxm-server`. By default it runs a
direct server-side ASK turn. Pass `--agent claude` to make each turn spawn and
communicate with an ACP Claude profile instead; `--agent-model` requests a
specific model when the selected ACP profile supports model control:

```bash
dekk apxm chat --agent claude --agent-model claude-3-5-haiku-latest
```

Use `workflow run` for checked-in `.apxmw` files.

## Key Exports

- `Cli` -- top-level Clap parser
- `Commands` -- enum of all subcommands

## Dependencies

| Crate | Purpose |
|-------|---------|
| apxm-driver | Compilation and execution coordination (behind `driver` feature) |
| apxm-core | Shared workflow contract, types, and error codes |
| apxm-compiler | `AirModule` for validation and codegen |
