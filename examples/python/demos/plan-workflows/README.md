# Open-Ended Workflow Dispatch Demo

The shortest path from "user types a sentence" to "APXM runs a
parallel workflow planned by an LLM and returns the result."

This demo exposes open-ended workflow dispatch as the user-facing shape.
The internal runtime still emits `PlanWorkflowEmitted` and links a task DAG into
the live execution DAG, but operators interact with a workflow request and a
traceable execution result.

## What this demo does

```
                         dispatch_open_ended.py
                                  │
                                  ▼
                ┌──────────────────────────────────┐
                │  1-PLAN-node workflow (compile)  │
                │   plan(goal=<user request>)      │
                └──────────────┬───────────────────┘
                               │  apxm.run()
                               ▼
                ┌──────────────────────────────────┐
                │  PLAN handler (plan.rs)          │
                │   1. call planner LLM            │
                │   2. parse response (inner_plan) │
                │   3. validate task_dag           │
                │   4. emit PlanWorkflowEmitted    │
                │   5. link_task_dag → ExecDag     │
                │   6. splice into live execution  │
                │   7. run spliced workflow        │
                └──────────────┬───────────────────┘
                               │
                               ▼
                          ExecutionResult
```

The full runtime, including round-robin backend dispatch, prefix-cache
hints, scheduler priority, and reproducible session traces, applies to
the spliced workflow just like any static skill workflow.

## Prerequisites

Follow [`docs/backends/model-zoo-quickstart.md`](../../../../docs/backends/model-zoo-quickstart.md)
to stand up a vLLM zoo. You need:

- `APXM_VLLM_HF_HOME` exported.
- One vLLM backend registered in your APXM config (the demo uses the
  configured benchmark backend or `APXM_BENCHMARK_BACKEND`).

## Running

```bash
# Smoke (no LLM — print the AIR the demo would dispatch).
python3 examples/python/demos/plan-workflows/dispatch_open_ended.py --dry-run

# With a custom request:
python3 examples/python/demos/plan-workflows/dispatch_open_ended.py \
    "draft a 2-paragraph explainer of paged attention then summarize it in one sentence"

# Through a service-exec (preferred for cluster use — runs inside the
# Slurm allocation so the loopback endpoint resolves):
dekk apxm vllm service-exec <SERVICE_NAME> -- \
    python3 examples/python/demos/plan-workflows/dispatch_open_ended.py \
    "your request here"
```

## What to look for in the trace

The PLAN handler emits `PlanWorkflowEmitted` only when the planner LLM
returns an `inner_plan.task_dag`. Trace-grep for it:

```bash
rg -n 'PlanWorkflowEmitted|parallel_fanout_max' <session-dir>/trace.ndjson
```

`parallel_fanout_max` is the largest set of tasks sharing the same
`depends_on` — i.e. the extracted parallelism width. A value of 1
means the LLM produced a linear chain; >1 means the LLM identified
true parallel branches the runtime can exploit.

## What this demo is NOT

- Not a replacement for static-skill authoring. Static skills are
  still the right shape for known repeating workflows. This demo is
  for open-ended requests where the structure is decided at runtime
  by the planner LLM.
- Not a guarantee the LLM returns a useful DAG. The validator hard-
  rejects cycles / dangling deps / dup ids with an actionable error,
  but garbage-in still produces garbage-out at the task-description
  level. Use a capable planner model.
- Not the full Rust `dispatch_open_ended()` helper
  scopes — that's a thin wrapper for ACP / external callers. This
  Python entry is the equivalent for human operators and notebook
  workflows.

## Cross-references

  (workspace-only, gitignored)
- PLAN handler source:
  [`plan.rs`](../../../../crates/runtime/apxm-runtime/src/executor/handlers/plan.rs)
- PLAN prompt:
  [`plan_outer_system.md.jinja`](../../../../crates/runtime/apxm-backends/prompts/plan_outer_system.md.jinja)
- `TaskDag::validate`:
  [`task.rs`](../../../../crates/core/apxm-core/src/types/execution/task.rs)
- `PlanWorkflowEmittedPayload`:
  [`payload.rs`](../../../../crates/core/apxm-core/src/events/payload.rs)
