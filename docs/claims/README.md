# APXM claim cards

This directory holds **claim cards** — small, evidence-bound documents that
back paper-publishable assertions about APXM behaviour. Each card cites the
preregistration that authorized the run, the artifact path under
`.apxm/evaluation/<scenario>/runs/<UTC>/` that supplies the evidence, and
the write-up under `docs/evaluation/<scenario>/` that interprets it.

## The chain

```
docs/preregistrations/<UTC>-<descriptor>.md     (frozen, append-only)
    │
    ▼  drives one or more runs
.apxm/evaluation/<scenario>/runs/<UTC>/         (raw artifacts; gitignored)
    │
    ▼  interpreted by
docs/evaluation/<scenario>/<writeup>.md          (narrative + tables)
    │
    ▼  promoted to a paper-bound claim by
docs/claims/<id>.md                              (this directory)
```

`apxm-finish` refuses to claim completion of a claim-bearing run if the
preregistration commit isn't present. `apxm-claim-evidence` is the skill
that drafts and audits claim cards.

## Card format

Each card lives at `docs/claims/<id>.md` with one section per required
field:

```markdown
# Claim <id>: <one-line statement>

## Assertion
<the claim as it would appear in the paper, in present tense>

## Scope
<workload, hardware, software versions, N, concurrency, time bounds>

## Evidence
- Preregistration: <SHA> <path>
- Artifact: <path under .apxm/evaluation/>
- Write-up: <path under docs/evaluation/>

## Statistical treatment
<test, alpha, power, effect size, confidence interval>

## Threats to validity
<concrete; not "we tried hard">

## Reproduction
<exact dekk command + env vars + expected artifact path>
```

## Status

Claim cards are created as paper-bound claims land. Early branches may have
no cards yet; that is expected and not a defect.

For ongoing campaign status (which preregistrations are in flight, which
runs have completed), see `docs/evaluation/README.md`.
