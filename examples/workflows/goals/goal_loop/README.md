# Goal Loop Workflow Pack

This pack shows the APXM-owned goal loop as a workflow artifact without hiding a
recursive prompt loop inside one model call.

The checked-in `workflow.apxmw` is deterministic and runnable without external
agents. It exercises one admitted pass through the control envelope:

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
[start pass through goal_start]
    |
    v
[eval]
    |
    +--> pass       -> record evidence + done
    +--> needs_more -> emit feedback event for another admitted pass
    +--> blocked    -> checkpoint / human resume
    +--> unsafe     -> cancel + record failure
```

The production action step represented by `start_pass.air` is the server MCP
call:

```text
goal_start({ task, context, event, trigger, workers, workspace, ... })
```

That call remains one explicit bounded worker DAG. If `eval` returns
`needs_more`, the controller or APXM OS starts another admitted pass with a new
request; it does not recursively prompt hidden workers outside APXM. The
deterministic AIR files label these transitions for testing. APXM server owns
execution IDs, session IDs, retained events, cancellation, worker admission, and
evidence for each pass; APXM OS or the calling controller owns external trigger
listeners, dedupe, retry, and re-arm behavior.

## Run The Deterministic Pack

From the repository root:

```bash
dekk apxm workflow validate examples/workflows/goals/goal_loop/workflow.apxmw
dekk apxm workflow analyze examples/workflows/goals/goal_loop/workflow.apxmw
dekk apxm workflow run examples/workflows/goals/goal_loop/workflow.apxmw \
  goal="ship a bounded APXM improvement" \
  event="manual goal requested" \
  policy="goal_loop.policy.json"
```

The workflow output is a traceable string that names the trigger, bounded pass,
start-pass action, eval, and feedback decision.

## Native Pass Requests

Use `deterministic_pass_request.json` as a no-agent `goal_start`
request. Use `acp_git_worktree_pass_request.json` after replacing the profile
IDs with registered APXM worker profiles and granting:

```json
["SPAWN_AGENT"]
```

The schema in `pass_request.schema.json` documents the expected shape for one
bounded pass. `goal_loop.policy.json` documents the outer loop limits:
iteration budget, timeout, budget, cancellation, and checkpoint behavior that
the controller must enforce while APXM server executes each admitted pass.

## Boundary

- APXM skills/plugins are triggers and instructions, not the runtime.
- Worker-authored graphs are proposals until APXM validates and admits them.
- `goal_start` executes one bounded pass and returns workflow
  status/events/cancel handles.
- APXM OS owns external event listeners, trigger sidecars, dedupe, retry, and
  re-arm behavior.
- APXM server owns execution IDs, session IDs, retained events, cancellation,
  worker admission, and evidence.
