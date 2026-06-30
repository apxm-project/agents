# Goal Loop Workflow Pack

This pack shows the APXM-owned goal loop as a workflow artifact without hiding a
recursive prompt loop inside one model call.

The checked-in `workflow.apxmw` is deterministic and runnable without external
agents. It exercises the external trigger and policy envelope before native
goal execution:

```text
[goal/event]
    |
    v
[trigger + policy gate]
    |
    v
[bounded pass request]
    |
    v
[start goal through goal_start]
    |
    v
[eval]
    |
    +--> pass       -> record evidence + done
    +--> needs_more -> native goal continues if iteration budget remains
    +--> blocked    -> checkpoint / human resume
    +--> unsafe     -> cancel + record failure
```

The production action step represented by `start_pass.air` is the server MCP
call:

```text
goal_start({ task, context, event, trigger, planning, workspace, ... })
```

When `workers` is omitted, `goal_start` creates a bounded worker workflow for each
pass; when `workers` is present, that explicit worker workflow is used. If the gate returns
`needs_more`, APXM server starts the next pass while iteration budget remains;
it does not recursively prompt hidden workers outside APXM. The deterministic
AIR files label these transitions for testing. APXM server owns goal IDs,
execution IDs, session IDs, retained events, cancellation, worker admission, and
evidence for each pass; APXM OS or the calling controller owns external trigger
listeners, dedupe, retry, and re-arm behavior.

## Run The Deterministic Pack

From the repository root:

```bash
dekk agents workflow validate examples/workflows/goals/goal_loop/workflow.apxmw
dekk agents workflow analyze examples/workflows/goals/goal_loop/workflow.apxmw
dekk agents workflow run examples/workflows/goals/goal_loop/workflow.apxmw \
  goal="ship a bounded APXM improvement" \
  event="manual goal requested" \
  policy="goal_loop.policy.json"
```

The workflow output is a traceable string that names the trigger, bounded pass,
start-pass action, eval, and feedback decision.

## Native Goal Requests

Use `deterministic_pass_request.json` as a no-agent `goal_start`
request. Use `acp_git_worktree_pass_request.json` after replacing the profile
names with resolvable APXM ACP profiles and granting:

```json
["grant_spawn_agent_example"]
```

The schema in `pass_request.schema.json` documents the expected shape for one
bounded pass. Omit `workers` for server auto-planning, or provide it to pin the
workflow manually. `goal_loop.policy.json` documents the outer loop limits:
iteration budget, timeout, budget, cancellation, and checkpoint behavior that
the controller declares while APXM server executes the admitted goal.

## Boundary

- APXM skills/plugins are triggers and instructions, not the runtime.
- Worker-authored workflows are proposals until APXM validates and admits them.
- `goal_start` starts a server-owned goal and returns `goal_id` plus
  `goal_status/events/cancel` handles.
- APXM OS owns external event listeners, trigger sidecars, dedupe, retry, and
  re-arm behavior.
- APXM server owns goal IDs, execution IDs, session IDs, retained events,
  cancellation, worker admission, and evidence.
