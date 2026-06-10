# Autonomous Task Goal Run

`goal_start` lets an agent stop manually prompting subagents. The
controller submits a task, optionally with an explicit bounded worker plan.
When `workers` is omitted, APXM creates the bounded worker DAG for the pass.
APXM materializes a workflow, starts it in the background, and the controller sleeps until
`workflow_status`, `workflow_events`, or `workflow_cancel` wakes
it.

```text
[event/task]
    |
    v
[trigger + optional explicit worker plan]
    |
    v
[allocate per-worker workspace/worktree]
    |
    v
[start background workflow]
    |
    +--> [worker A SPAWN_AGENT -> COMMUNICATE]
    +--> [worker B SPAWN_AGENT -> COMMUNICATE]
    +--> [worker N SPAWN_AGENT -> COMMUNICATE]
    |
    v
[gate/eval waits for all workers]
    |
    v
[feedback]
    |
    v
[sleep until status/events/cancel wakes the caller]
```

Use `deterministic_request.json` when you want to exercise the workflow shape
without any external agent profiles. Use `acp_git_worktree_request.json` when
the runtime has registered ACP profiles such as `codex`, `claude`, or any
custom worker profile. APXM is profile-name agnostic; those names are examples.

For CLI callers, `dekk apxm goal` is the high-level wrapper around this native
MCP path. By default it lets the server plan the bounded worker request, calls
`goal_start`, and follows `workflow_events/status` unless `--no-follow` is set.
Pass repeatable `--worker` and `--depends` only when the worker DAG must be
pinned manually.

The returned JSON includes:

- `execution_id` for `workflow_status/events/cancel`.
- `workflow_path` and `bundle_dir` for the generated workflow bundle.
- `artifacts.tracking_doc`, `artifacts.graph_json`, `artifacts.plan_json`,
  `artifacts.worker_prompts[*].prompt`, and initialized report files for the
  generated goal packet.
- `plan.workers[*].cwd` showing each worker's assigned workspace.
- `planning` showing whether APXM generated the worker DAG or the caller
  provided it explicitly.
- `goal.next_events_args` with the first `workflow_events` cursor.
- `goal.sleep_event_kind = "orchestrator_sleep"` and
  `goal.wake_event_kind = "orchestrator_wake"`.
- `goal_prompt` describing the autonomous sleep/wake loop.

The run event stream includes an `orchestrator_sleep` event once APXM has
accepted ownership of the workflow and an `orchestrator_wake` event before the
terminal `execute_complete`, `error`, or `turn_aborted` event. The controller
agent should not prompt workers manually after start; it should page
`workflow_events` with `since = next_seq`, confirm the terminal state with
`workflow_status`, and only start another bounded pass if the feedback step
requires it.

Worker, supervisor, tracking, default-instruction, and goal prompt text
is rendered from Markdown templates in `apxm-backends/prompts/`, not hardcoded
inside the server. Project or user overrides can replace those templates through
the normal APXM prompt override paths.

Naming rule: public MCP tool names are prefixless because the MCP server name
already provides the APXM namespace. Generated packet titles, workflow names,
prompt headings, and report headings stay neutral for the same reason.

For real ACP workers, callers must grant process spawning explicitly:

```json
"admit_capabilities": ["SPAWN_AGENT"]
```

In `git_worktree` mode, APXM runs `git worktree add --detach` once per worker
under the generated bundle directory and passes that path as `cwd` to
`SPAWN_AGENT`. The MVP keeps worktrees for review.
