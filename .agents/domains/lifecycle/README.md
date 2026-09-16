# Domain — lifecycle

The six lifecycle skills are lightweight guidance for planning, execution,
verification, and delivery. Use the subset that matches the task:

1. **context** — prime the session (doctor, AGENTS.md,
   `_shared/` rules, subsystem ownership).
2. **plan** — design before implementing; required for >3 files,
   public API changes, or Slurm allocations.
3. **execute-plan** — drive a written plan with focused verification,
   no scope creep.
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
  scan, check-agent-skills).

## Related rules

- `_shared/apxm-agent-operating-rules.md` — commit discipline,
  Slurm safety.
- `_shared/apxm-development-rules.md` — authority CLI, build env.
