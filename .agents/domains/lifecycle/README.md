# Domain — lifecycle

The 6-skill workflow backbone. Every non-trivial session ceremonially
routes through these:

1. **context** — prime the session (doctor, project.md,
   `_shared/` rules, subsystem ownership).
2. **plan** — design before implementing; required for >3 files,
   public API changes, or Slurm allocations.
3. **execute-plan** — drive the plan with the current harness task
   tracker, focused verification, no scope creep.
4. **simplify** — remove copied `_shared/` text, weak
   abstractions, referential comments before claiming done.
5. **finish** — focused tests, doctor, secrets scan, and artifact
   placement.
6. **commit** — commit/push gate; no push without approval, push to
   `main` only when explicitly authorized.

## When to skip the workflow

- Typo fix or single-line edit: skip `context` and `plan`;
  still run `finish` + `commit`.
- Doc-only edit: skip `plan`; `finish` still runs (secrets
  scan, skills status).

## Related rules

- `_shared/apxm-agent-operating-rules.md` — commit discipline,
  Slurm safety.
- `_shared/apxm-development-rules.md` — authority CLI, build env.
