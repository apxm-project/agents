---
name: plan
group: Lifecycle
description: Produce a written plan before non-trivial APXM implementation. Required for changes touching >3 files, modifying a public API or AIS op, or needing Slurm GPU allocation. Enforces APXM-specific gates (AIS-op-vs-compose decision, dialect-codegen impact).
user-invocable: true
---

# APXM Plan

Write a plan before implementing. Mirrors native plan mode but enforces
APXM gates that generic planning skips.

## When this skill is required

- Change touches **>3 files**.
- Change modifies a **public API or AIS op** (anything other crates or
  the Python frontend will see).
- Change requires a **Slurm GPU allocation**.

Trivial bug fixes, doc-only edits, single-file refactors with a clear
local scope do not require this skill.

## What this skill does

1. **List affected files and subsystems**. Be specific: file paths,
   public API surface, downstream consumers.
2. **Decide AIS-op-vs-compose** if the change adds or modifies behavior
   currently expressed in `apxm-ais`. Adding an op? Invoke
   `ais-op-design` first — it owns the design-before-code gate.
3. **State expected verification**: which focused Dekk tests and checks,
   and which strict lints.
4. **State boundaries** — what the change is **NOT** doing. Prevents
   scope creep during execution.
5. **State a rollback plan** — branch name, what's reversible, what's
   not (e.g. codegen output, manifests committed mid-flight).
6. **Record the plan** in the task update or handoff before implementation.
   Existing task authorization is sufficient; ask only if the requested
   scope or an irreversible external action is genuinely unclear.

## Plan template

```markdown
## Goal
<one paragraph, concrete>

## Affected
- Files: <list>
- Subsystems: <crate(s), AIS ops, python frontend, runtime>
- Public API impact: <none / additive / breaking>

## Approach
<bullets>

## Pre-work
- [ ] ais-op-design (if adding an AIS op)
- [ ] dekk agents build-dialect + codegen (if editing .td)

## Verification per phase
- Phase 1: <command>
- Phase 2: <command>

## Boundaries — explicitly NOT in scope
- <bullet>

## Rollback
- Branch: <name>
- Reversible? <yes/no, plus what's not>
```

## Anti-patterns

- Plans that double as implementations ("First I'll edit X to add the
  field, then…"). Plans state the shape, not the diff.
- Plans without verification per phase.
- Plans that omit the boundaries section — scope creep happens precisely
  where boundaries aren't drawn.
- "Add new AIS op" plans that skip `ais-op-design`.

## Next step

Once the plan is recorded, invoke `execute-plan` when the change needs phased
work. Do not add another approval step to already-authorized work.
