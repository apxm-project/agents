# Autonomous Task Orchestration

`apxm_orchestrate_start` lets an agent stop manually prompting subagents. The
orchestrator submits one bounded worker plan, APXM materializes a workflow,
starts it in the background, and the orchestrator sleeps until
`apxm_workflow_status`, `apxm_workflow_events`, or `apxm_workflow_cancel` wakes
it.

```text
[event/task]
    |
    v
[trigger + bounded worker plan]
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
[sleep until status/events/cancel wakes the orchestrator]
```

Use `deterministic_request.json` when you want to exercise the workflow shape
without any external agent profiles. Use `acp_git_worktree_request.json` when
the runtime has registered ACP profiles such as `codex`, `claude`, or any
custom worker profile. APXM is profile-name agnostic; those names are examples.

For CLI callers, `dekk apxm goal` is the high-level wrapper around this native
MCP path. It builds the bounded worker request, calls `apxm_orchestrate_start`,
and follows `apxm_workflow_events/status` unless `--no-follow` is set.

The returned JSON includes:

- `execution_id` for `apxm_workflow_status/events/cancel`.
- `workflow_path` and `bundle_dir` for the generated workflow bundle.
- `artifacts.tracking_doc`, `artifacts.graph_json`, `artifacts.plan_json`,
  `artifacts.worker_prompts[*].prompt`, and initialized report files for the
  generated orchestration packet.
- `plan.workers[*].cwd` showing each worker's assigned workspace.
- `orchestration.next_events_args` with the first `apxm_workflow_events` cursor.
- `orchestration.sleep_event_kind = "orchestrator_sleep"` and
  `orchestration.wake_event_kind = "orchestrator_wake"`.
- `orchestrator_prompt` describing the autonomous sleep/wake loop.

The run event stream includes an `orchestrator_sleep` event once APXM has
accepted ownership of the workflow and an `orchestrator_wake` event before the
terminal `execute_complete`, `error`, or `turn_aborted` event. The orchestrator
agent should not prompt workers manually after start; it should page
`apxm_workflow_events` with `since = next_seq`, confirm the terminal state with
`apxm_workflow_status`, and only start another bounded pass if the feedback step
requires it.

Worker, supervisor, tracking, default-instruction, and orchestrator prompt text
is rendered from Markdown templates in `apxm-backends/prompts/`, not hardcoded
inside the server. Project or user overrides can replace those templates through
the normal APXM prompt override paths.

Naming rule: keep `apxm_` on public MCP tool names, because clients share a
global tool namespace. Generated packet titles, workflow names, prompt headings,
and report headings stay neutral because their bundle location already provides
the APXM context.

For real ACP workers, callers must grant process spawning explicitly:

```json
"admit_capabilities": ["SPAWN_AGENT"]
```

In `git_worktree` mode, APXM runs `git worktree add --detach` once per worker
under the generated bundle directory and passes that path as `cwd` to
`SPAWN_AGENT`. The MVP keeps worktrees for review.
