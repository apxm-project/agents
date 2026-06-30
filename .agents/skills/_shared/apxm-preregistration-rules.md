# Shared rule — APXM preregistration rules

Load before any session that will run a claim-bearing benchmark or
evaluation (see `apxm-evaluation-rules` for the definition).

## Norm

Every claim-bearing run requires a **committed** preregistration
*before* the run starts. The preregistration freezes the protocol — what
you're measuring, how you're measuring it, what counts as success — so
the result can't be reverse-engineered from the data.

## File layout

- Location: `workspace/eval/preregistrations/<UTC>-<descriptor>.md`.
- Naming: `<YYYYMMDDTHHMMSS>Z-<plan>-<scenario>-<arm>.md`.
  Example: `20260521T040539Z-apxm-priority-lane-c16-bg16.md`.
- Append-only. Once committed, do not edit. If the protocol changes,
  write a new file with a `-corrective`, `-restart`, `-iter<N>`, or
  similar suffix and reference the prior commit in the body.

## Template (minimum required sections)

```markdown
# <Plan> — <Scenario> (<UTC>Z)

## Goal
What hypothesis or claim does this run back?

## Hardware & service
- Service: <name from zoo.toml>
- Service allocation: <slurm job id, set at run time>
- GPU: <N x model>
- APXM commit: <set at run time>
- vLLM commit: <external/vllm submodule SHA>

## Protocol
- Workload: <reference to examples/python/benchmarks/workloads/...>
- Arms: A=<baseline>, B=<apxm>
- Iterations: <N>
- Concurrency: <c>
- Cache salt scope: <(arm, opt) — see apxm-evaluation-rules>
- Seed: <S>

## Success criteria
- Primary metric: <e.g. p99 dispatch latency>
- Effect size threshold: <e.g. >=15% improvement at p<0.05>
- Secondary metrics: <list>

## Exclusions / known confounds
- <list>

## Artifact location
`.apxm/evaluation/<scenario>/runs/<UTC>/`

## Linked outputs (filled after run)
- Write-up: workspace/eval/evidence/reports/<scenario>/<UTC>.md
- Claim card: eval repo evidence bundle (if paper-bound)
```

## When to invoke this skill

The `apxm-preregistration` skill walks you through:

1. Drafting the file from the template.
2. Confirming the protocol with the user (mandatory before commit).
3. Committing — `prereg(planNN): <descriptor>` per the repo log style.
4. Confirming the commit SHA was recorded before the run starts.

Skipping any of these makes the run unable to back a claim, regardless
of how good the numbers look.

## `finish` interlock

`finish` will refuse to claim "done" on a claim-bearing run if:

- No matching preregistration exists in `workspace/eval/preregistrations/`.
- The preregistration's commit timestamp is *after* the first artifact
  in `.apxm/evaluation/<scenario>/runs/<UTC>/`.
- The write-up doesn't cite the preregistration commit SHA.

## Existing examples

See `workspace/eval/preregistrations/` for the existing corpus (Plan 04 cross
system, Plan 05 tau2 airline/retail/telecom, Plan 09 J/req, priority
lane c=16/c=32, review council). Read 2–3 before drafting your own to
match style.
