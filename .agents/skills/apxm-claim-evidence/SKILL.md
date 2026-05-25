---
name: apxm-claim-evidence
description: Use after a claim-bearing run completes. Writes the docs/evaluation/ write-up, promotes a claim card under docs/claims/ if paper-bound, and links the preregistration + artifact paths.
user-invocable: true
---

# APXM Claim Evidence

Load `_shared/apxm-evaluation-rules.md` and
`_shared/apxm-preregistration-rules.md` before broad work.

## When this skill applies

A claim-bearing run has completed. Artifacts exist under
`.apxm/evaluation/<scenario>/runs/<UTC>/`. A preregistration is already
committed in `docs/preregistrations/`. Now you need to:

1. Write the run-up under `docs/evaluation/<scenario>/<UTC>.md`.
2. (If paper-bound) promote a claim card under `docs/claims/<id>.md`.

## Write-up template

```markdown
# <Plan> — <Scenario> — <UTC>Z

## Preregistration
docs/preregistrations/<prereg-file>.md
Commit SHA: <sha>

## Setup
- APXM commit: <sha>
- vLLM commit: <external/vllm sha>
- Service: <name> (Slurm job <id>)
- Hardware: <N x model>

## Method
Brief summary; defer to preregistration for protocol details.

## Results
- Primary metric: <value>
- Effect size: <value, CI>
- Secondary metrics: <table>

## Artifacts
`.apxm/evaluation/<scenario>/runs/<UTC>/`
- `manifest.json`
- `<raw outputs>`

## Interpretation
What the numbers do and do not support.

## Known confounds / limitations
From the preregistration; updated if anything new surfaced.

## Follow-ups
- <list>
```

## Claim card template (`docs/claims/<id>.md`)

```markdown
# Claim <id>: <one-sentence claim>

## Status
<draft | verified | retracted>

## Backing run
docs/evaluation/<scenario>/<UTC>.md

## Preregistration
docs/preregistrations/<prereg-file>.md (commit <sha>)

## Numbers
- <metric>: <value> (<CI>)

## Scope of the claim
Where it applies; where it does not.

## Replication
Exact commands to reproduce from the artifact directory.
```

## Steps

1. Write the run-up.
2. (If paper-bound) promote a claim card. Cross-link prereg ↔ write-up
   ↔ claim card via filenames and commit SHAs.
3. Commit:
   `eval(<scenario>): <UTC> write-up` or
   `claim(<id>): <descriptor>`.
4. Confirm `apxm-finish`'s preregistration interlock is satisfied
   (timestamp, SHA in write-up).

## Anti-patterns

- Editing a committed claim card silently. Add a "Retracted" section
  or write a superseding card and update the status.
- Writing the run-up without citing the preregistration commit SHA.
- Burying confounds — surface them, even if they weaken the headline
  number.
