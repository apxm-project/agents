---
name: apxm-goal-orchestrator
description: Use when an agent should turn a complex APXM goal into a bounded worker DAG or workflow, execute it through APXM, wait on workflow events/status, and synthesize verified artifacts. Covers `dekk apxm goal`, `goal_start`, `workflow_*`, and `prompt_as_workflow` selection.
user-invocable: true
---

# APXM Goal Orchestrator

Load `_shared/apxm-development-rules.md` before broad work.

Use this skill when a human or agent wants APXM to own a complex pass instead
of manually prompting subagents. The planner/orchestrator creates one explicit
bounded pass, APXM executes it, and the caller waits through APXM events.

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
  --repo-root /path/to/repo \
  --worker research:"Inspect relevant code and docs":worker-a \
  --worker implement:"Make the scoped patch":worker-b \
  --worker verify:"Run checks and inspect evidence":worker-c \
  --depends implement=research \
  --depends verify=implement \
  --supervisor worker-d
```

Profile IDs are examples. Bind roles to whatever APXM-registered workers are
ready; do not assume Claude, Codex, or any provider-specific host exists.

Use `--event` and `--trigger` when this pass comes from an external event, and
`--dry-run` when the first step should only materialize and validate the bundle.

## MCP Pattern

1. Call `goal_start` once with `task`, optional
   `context/event/trigger`, explicit `workers`, optional `supervisor`, and
   workspace policy. Include `admit_capabilities: ["SPAWN_AGENT"]` for real
   ACP/headless workers.
2. Store `execution_id`, `session_id`, `session_dir`, `workflow_path`,
   `bundle_dir`, and returned artifact paths.
3. Stop prompting workers manually. Page `workflow_events` with
   `since = next_seq`; wake on `orchestrator_wake` or terminal events.
4. Confirm the terminal result with `workflow_status`.
5. Use `workflow_cancel` for interruption. Do not invent a second cancel
   or process-control path for server-owned runs.

## Worker DAG Rules

- Split by independent artifacts: research, implementation, critique,
  verification, and synthesis are roles, not provider names.
- Use `--depends` or `depends_on` to create phases. Keep each pass bounded;
  if feedback requires more work, start another admitted pass.
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

Return the execution id, status, worker roles/profiles used, workflow/session
paths, generated artifacts, verification evidence, warnings, and the next
bounded pass only if feedback requires one.
