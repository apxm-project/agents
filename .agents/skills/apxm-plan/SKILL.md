---
name: apxm-plan
description: Produce a written plan before non-trivial APXM implementation. Required for changes touching >3 files, modifying a public API or AIS op, introducing a claim, or needing Slurm GPU allocation. Enforces APXM-specific gates (preregistration, AIS-op-vs-compose decision, dialect-codegen impact).
user-invocable: true
---

# APXM Plan

Write a plan before implementing. Mirrors native plan mode but enforces
APXM gates that generic planning skips.

## When this skill is required

- Change touches **>3 files**.
- Change modifies a **public API or AIS op** (anything other crates or
  the Python frontend will see).
- Change is **claim-bearing** (output backs a benchmark, paper, or
  "X is faster than Y" assertion).
- Change requires a **Slurm GPU allocation**.
- Change rebases or edits `external/vllm`.

Trivial bug fixes, doc-only edits, single-file refactors with a clear
local scope do not require this skill.

## What this skill does

1. **List affected files and subsystems**. Be specific: file paths,
   public API surface, downstream consumers.
2. **Decide AIS-op-vs-compose** if the change adds or modifies behavior
   currently expressed in `apxm-core`. Adding an op? Invoke
   `apxm-ais-op-design` first — it owns the design-before-code gate.
3. **Decide claim-bearing?** If yes, invoke `apxm-preregistration`
   first. The preregistration must be committed before the run starts.
4. **State expected verification**: which `dekk apxm test -p <crate>`,
   which integration test, which `dekk apxm vllm zoo-status` probe,
   which `--strict` lint.
5. **State boundaries** — what the change is **NOT** doing. Prevents
   scope creep during execution.
6. **State a rollback plan** — branch name, what's reversible, what's
   not (e.g. codegen output, manifests committed mid-flight).
7. **Get user sign-off** — use the harness's plan-approval surface
   (Claude Code: `ExitPlanMode`) or an explicit confirmation from the
   user in conversation. Do not proceed without it.

## Plan template

```markdown
## Goal
<one paragraph, concrete>

## Affected
- Files: <list>
- Subsystems: <crate(s), AIS ops, python frontend, vLLM fork>
- Public API impact: <none / additive / breaking>

## Approach
<bullets>

## Pre-work
- [ ] apxm-ais-op-design (if adding an AIS op)
- [ ] apxm-preregistration (if claim-bearing)
- [ ] dekk apxm build-dialect + codegen (if editing .td)

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
- "Add new AIS op" plans that skip `apxm-ais-op-design`.
- Claim-bearing plans without preregistration in the pre-work checklist.

## Next step

Once the user signs off, invoke `apxm-execute-plan`. Do not begin
implementation before approval.
