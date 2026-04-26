---
name: execute
description: Compile and run a graph in one step
user-invocable: true
---

# Execute

Compiles an AIR graph and immediately runs it through the dataflow scheduler — the "compile and run" shortcut. This is the primary command for development: you edit your graph, run `execute`, and see results. Under the hood it performs the full compilation pipeline (parse, lower to MLIR, optimize, emit artifact) and then hands the artifact to the runtime executor.

The runtime uses a parallel dataflow scheduler that executes independent nodes concurrently, routes data tokens between operations, manages LLM backend calls, and handles retries and timeouts.

## Commands

```bash
dekk apxm execute graph.air                           # compile (O2) and run
dekk apxm execute graph.air -O0                       # skip optimizations (useful for debugging)
dekk apxm execute graph.air -O2                       # standard optimizations before execution
dekk apxm execute graph.air --emit-metrics m.json     # write runtime statistics after execution
dekk apxm execute graph.air -- "what is APXM?"        # pass arguments to the entry flow
```

## How It Differs from Compile + Run

`execute` is equivalent to `compile` followed by `run`, but in a single process — no intermediate artifact file is written to disk. Use `execute` during development for the edit-run loop. Use separate `compile` then `run` when you want to pre-compile once and execute many times, or when you need the artifact for distribution.

## Runtime Execution

Once compilation completes, the runtime:

1. **Initializes** the LLM registry (loads credentials from `~/.apxm/credentials.toml`), capability registry, sandbox policies, and agent profiles
2. **Finds the @entry flow** in the compiled artifact — this is the top-level workflow
3. **Validates arguments** against the entry flow's parameter list (count and order must match)
4. **Schedules execution** using a parallel dataflow scheduler:
   - Nodes with no dependencies start immediately
   - Data tokens flow along edges as nodes complete
   - Independent branches execute concurrently (parallelism bounded by available backends)
   - `WAIT_ALL`, `MERGE`, `FENCE` nodes synchronize parallel branches
5. **Returns results** from exit nodes, plus execution statistics

## Metrics Output

With `--emit-metrics`, writes a JSON report after execution:

- **Execution stats**: nodes executed/failed, total duration, per-node timing
- **Scheduler metrics**: per-op overhead, work-stealing time, max/avg parallelism
- **LLM metrics**: token counts, latencies, backend utilization (when `metrics` feature enabled)

## When to Use

- Primary development workflow: edit graph, execute, inspect results
- When you don't need a persistent compiled artifact
- When testing graph changes iteratively
- Use `dekk apxm compile` + `dekk apxm run` instead for production or repeated execution
