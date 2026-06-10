# Native Goal Workflows

These examples exercise APXM's native workflow coordination surface for
agent-agnostic worker graphs. The checked-in workers are deterministic AIR
graphs, so they run without Claude, Codex, API keys, ACP profiles, or network
access. Replace any worker graph with a graph, artifact, or workflow that calls
a registered agent when you want the same shape to drive real workers.

## Three Ways To Run Complex Work

Use the smallest surface that matches the job:

- `dekk apxm goal`: an agent or user creates one bounded worker DAG, APXM
  materializes the workflow bundle, starts it in the background, and wakes the
  caller through `workflow_events` and `workflow_status`.
- `dekk apxm workflow run`: run a checked-in
  `.apxmw` workflow file after `validate` and `analyze`.
- `prompt_as_workflow`: ask MCP to synthesize a typed APXM workflow from natural
  language. It is workflow-oriented; worker-spawning goal execution should still go
  through `goal` or `goal_start`.

```text
[goal or event]
       |
       v
[planner/controller creates bounded DAG]
       |
       v
[APXM starts workflow + records execution_id]
       |
       v
[caller sleeps]
       |
       v
[workers finish -> gate/eval -> feedback]
       |
       v
[events/status wake caller]
```

## Autonomous Task

`autonomous_task/` shows the native MCP path for an controller agent that
creates a bounded parallel worker graph, assigns each worker a workspace or Git
worktree, starts the workflow in the background, and then sleeps until APXM
status/events/cancel wakes it.

```text
[event/task] -> [trigger] -> [parallel workers] -> [gate/eval] -> [feedback]
                                      |
                                      v
 [workflow_status + workflow_events + workflow_cancel]
```

Use `autonomous_task/deterministic_request.json` for a no-agent smoke test, or
`autonomous_task/acp_git_worktree_request.json` with registered ACP profiles.

## Goal Loop

`goal_loop/` shows the APXM-owned control envelope for a long-running goal
without pretending `.apxmw` has recursive scheduler loops. It turns one admitted
goal event into one bounded goal pass, then records feedback that can
trigger another admitted pass through APXM OS or an MCP client.

```text
[goal/event] -> [trigger + policy gate] -> [bounded pass request]
                                             |
                                             v
                                      [start one pass]
                                             |
                                             v
                                    [eval] -> [feedback]
```

Run the deterministic pack from the repository root:

```bash
dekk apxm workflow validate examples/workflows/goals/goal_loop/workflow.apxmw
dekk apxm workflow analyze examples/workflows/goals/goal_loop/workflow.apxmw
dekk apxm workflow run examples/workflows/goals/goal_loop/workflow.apxmw \
  goal="ship a bounded APXM improvement" \
  event="manual goal requested" \
  policy="goal_loop.policy.json"
```

## Agent Council

`agent_council/workflow.apxmw` fans a task out to three independent workers and
then fans their outputs into a synthesizer.

```text
                 [task]
                   |
       +-----------+-----------+
       |           |           |
  [planner]   [executor]   [reviewer]
       |           |           |
       +-----------+-----------+
                   |
             [synthesizer]
                   |
                [output]
```

Run it from the repository root:

```bash
dekk apxm workflow validate examples/workflows/goals/agent_council/workflow.apxmw
dekk apxm workflow analyze examples/workflows/goals/agent_council/workflow.apxmw
dekk apxm workflow run examples/workflows/goals/agent_council/workflow.apxmw task="ship native workflow coordination"
```

## Event Feedback Loop

`event_feedback_loop/workflow.apxmw` models a single deterministic pass through
event, trigger, action, eval, and feedback. A daemon, scheduler, or MCP client
can relaunch the workflow when feedback says another pass is needed.

```text
[event] -> [trigger]
              |
        +-----+-----+
        |           |
   [write action] [verify action]
        |           |
        +-----+-----+
              |
            [eval]
              |
          [feedback]
```

Run it from the repository root:

```bash
dekk apxm workflow run examples/workflows/goals/event_feedback_loop/workflow.apxmw event="repository changed"
```

## Approval Gate

`approval_gate/workflow.apxmw` parks on a checkpoint and wakes when the APXM
checkpoint endpoint resumes it. This is the parent-sleeps-while-worker-waits
path used for human approval, external events, and long-running background
agents.

```text
[workflow start] -> [RESUME checkpoint] --parks--> [checkpoint resume event]
                                           |
                                           v
                                      [final output]
```

MCP clients should use the native workflow tools:

```text
workflow_start  -> starts a background workflow and returns execution_id
workflow_status -> polls or inspects status by execution_id
workflow_events -> reads ordered run events with since/limit paging
workflow_cancel -> interrupts a running or parked workflow
```

For the approval example, create checkpoint `examples-approval-cp`, start the
workflow, then resume or cancel it through the server. The E2E tests in
`crates/tools/apxm-server/src/tests/mcp.rs` run these checked-in workflows
through `workflow_start`, `workflow_status`, `workflow_events`,
and `workflow_cancel`.

## Local Background Workflow

`cancel_background/background_ok.apxmw` is a fast deterministic workflow suited
for explicit local CLI background workflow testing when a server control plane
is not involved. `cancel_background/cancel_parked.apxmw` parks on checkpoint
`examples-cancel-cp` and is intended for testing cancellation through
`workflow_cancel` when launched through the native MCP workflow tools.

```bash
dekk apxm workflow run examples/workflows/goals/cancel_background/background_ok.apxmw \
  --background \
  --json
```
