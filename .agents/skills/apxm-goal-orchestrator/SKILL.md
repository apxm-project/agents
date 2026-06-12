---
name: apxm-goal-orchestrator
description: Use when an agent should turn a complex APXM goal into a bounded worker workflow, execute it through APXM, wait on goal events/status, and synthesize verified artifacts. Covers `dekk apxm goal`, `goal_start`, `goal_*`, workflow drill-down, and `prompt_as_workflow` selection.
user-invocable: true
---

# APXM Goal Orchestrator

Load `_shared/apxm-development-rules.md` before broad work. When the goal
authors or edits a self-hosted dev workflow, also load
`_shared/apxm-self-host-rules.md`.

Use this skill when a human or agent wants APXM to own a complex goal instead
of manually prompting subagents. `goal_start` can create bounded worker workflows
from the task, or accept explicit workers when the caller needs to pin it. APXM
executes each pass, continues when the gate asks for more work, and wakes the
caller through APXM events.

## Choose the Surface

1. **Goal orchestration**: use `dekk apxm goal` or MCP
   `goal_start` when the task needs worker roles, fan-out/fan-in,
   workspaces, worktrees, gate/eval, and sleep/wake events.
2. **Checked-in workflow**: use `dekk apxm workflow validate|analyze|run`
   when a `.apxmw` file already exists or the work should become a reusable
   workflow artifact.
3. **Workflow synthesis**: use `prompt_as_workflow` when natural language should
   become a typed APXM workflow. Treat generated workflows as proposals until APXM
   validates, compiles, and admits them. Do not use this path to bypass
   external-worker admission.

## CLI Pattern

```bash
dekk apxm agent list
dekk apxm goal "Investigate and implement the bounded change" \
  --context "Repo: /path/to/repo; constraints: focused patch + tests" \
  --workspace git_worktree \
  --repo-root /path/to/repo
```

By default the CLI omits `workers`, asks `goal_start` to auto-plan the bounded
worker workflow, and requests APXM agent selection. Use repeatable `--worker` and
`--depends` only when the pass must be pinned manually. Profile IDs are
examples; bind roles to whatever APXM-registered workers are ready, and do not
assume Claude, Codex, or any provider-specific host exists.

Use `--event` and `--trigger` when this pass comes from an external event, and
`--dry-run` when the first step should only materialize and validate the bundle.

## MCP Pattern

1. Call `goal_start` once with `task`, optional
   `context/event/trigger`, optional `planning`, optional explicit `workers`,
   optional `supervisor`, and workspace policy. Omit `workers` for server-owned
   auto-planning. Include `admit_capabilities: ["SPAWN_AGENT"]` for real
   ACP/headless workers.
2. Store `goal_id`, `session_id`, `session_dir`, `workflow_path`,
   `bundle_dir`, and returned artifact paths. Use the current `execution_id`
   only for workflow drill-down.
3. Stop prompting workers manually. Stream
   `/v1/goals/{goal_id}/events/stream`, or page `goal_events` with
   `since = next_seq` when using MCP; wake on aggregate `orchestrator_wake` or
   terminal goal status.
4. Confirm the terminal result with `goal_status`.
5. Use `goal_cancel` for interruption. Do not invent a second cancel or
   process-control path for server-owned runs.

## Worker Workflow Rules

- Split by independent artifacts: research, implementation, critique,
  verification, and synthesis are roles, not provider names.
- Let `goal_start` create the worker workflow unless the phase order must be pinned. Use
  `--depends` or `depends_on` to create manual phases. Keep each pass bounded;
  APXM starts the next pass when the gate asks for more work and the goal still
  has iteration budget.
- In `git_worktree` mode, provide `--repo-root`; APXM assigns per-worker
  detached worktrees and records them in the orchestration packet.
- Keep worker briefs compact: objective, relevant paths, constraints, expected
  artifact, verification criteria, budget/timeout, and stop conditions.
- Put new prompt/prose bodies in Markdown templates or skill docs, not
  hardcoded Rust strings.

## Anti-patterns

- Adding an MCP tool that combines natural-language planning, worker admission,
  execution, waiting, and policy into one opaque call.
- Manually spawning or reprompting workers after APXM accepted the workflow.
- Treating `prompt_as_workflow` output as trusted executable worker spawn logic.
- Starting raw shell background jobs for server-owned workflows.
- Claiming APXM verification when only local files were inspected.

## Done condition

Return the goal id, status, worker roles/profiles used, workflow/session
paths, generated artifacts, verification evidence, warnings, and any remaining
bounded next action if the goal did not converge.
