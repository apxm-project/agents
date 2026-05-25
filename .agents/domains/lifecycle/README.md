# Domain — lifecycle

The 6-skill workflow backbone. Every non-trivial session ceremonially
routes through these:

1. **apxm-context** — prime the session (doctor, project.md,
   `_shared/` rules, subsystem ownership).
2. **apxm-plan** — design before implementing; required for >3 files,
   public API changes, or Slurm allocations.
3. **apxm-execute-plan** — drive the plan with `TaskCreate`/
   `TaskUpdate`, focused verification, no scope creep.
4. **apxm-simplify** — remove copied `_shared/` text, weak
   abstractions, referential comments before claiming done.
5. **apxm-finish** — focused tests, doctor, no-legacy lint, secrets
   scan, artifact placement.
6. **apxm-commit** — pre-PR gate; no auto-commit, no push without
   approval, no push to main, no `--no-verify`.

## When to skip the workflow

- Typo fix or single-line edit: skip `apxm-context` and `apxm-plan`;
  still run `apxm-finish` + `apxm-commit`.
- Doc-only edit: skip `apxm-plan`; `apxm-finish` still runs (secrets
  scan, skills status).

## Related rules

- `_shared/apxm-agent-operating-rules.md` — commit discipline,
  Slurm safety.
- `_shared/apxm-development-rules.md` — authority CLI, build env.
