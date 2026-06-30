# Domain — evaluation

Benchmarks, preregistration, claim cards, and paper-bound evidence now live in
`apxm-project/eval`. This domain file is a pointer for APXM core agents
who encounter evaluation references while working in this repo.

## Skills

- **apxm-preregistration** — draft & commit a
  `docs/preregistrations/` entry before claim-bearing runs.
- **apxm-priority-lane-bench** — priority-lane benchmark workflow.
- **apxm-review-council-bench** — review-council benchmark workflow.
- **apxm-claim-evidence** — write-up + claim card after a run.
- **apxm-evaluation-artifacts** — artifact placement under `.apxm/`.

## Norm

Every claim-bearing run requires a **committed preregistration before
the run starts**. The current gate lives with the eval harness in
`apxm-project/eval`; APXM core does not host paper drafts or claim cards.

## Layout

Raw run artifacts written by APXM core land at
`.apxm/evaluation/<scenario>/runs/<UTC>/`. Preregistrations, claim
cards, and scenario write-ups live in `apxm-project/eval`; that
repo owns its own layout.

## Paired-arm benchmarks

Cache-salt scoping must be **per `(arm, opt)`**, never per
`(iter, row)`. See `feedback_paired_arm_cache_salt_scoping`.

## tau2 reminder

`tau2 --seed S --num-tasks N` is not nested across N. Cross-N ratio
tables are descriptive only — use `--task-ids` for nested arcs.

## Related rules

- `_shared/apxm-evaluation-rules.md`
- `_shared/apxm-preregistration-rules.md`
