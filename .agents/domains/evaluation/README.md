# Domain — evaluation

Benchmarks, preregistration, claim cards, and paper-bound evidence now live in
`apxm-project/apxm-eval`. This domain file is a pointer for APXM core agents
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
`apxm-project/apxm-eval`; APXM core does not host paper drafts or claim cards.

## Layout

- `apxm-project/apxm-eval/docs/preregistrations/` — frozen, append-only
  preregistrations.
- `apxm-project/apxm-eval/docs/claims/` — claim cards for paper-bound numbers.
- `apxm-project/apxm-eval/docs/evaluation/` — scenario write-ups.
- `.apxm/evaluation/<scenario>/runs/<UTC>/` — raw artifacts in the repo where
  the run is executed.

## Paired-arm benchmarks

Cache-salt scoping must be **per `(arm, opt)`**, never per
`(iter, row)`. See `feedback_paired_arm_cache_salt_scoping`.

## tau2 reminder

`tau2 --seed S --num-tasks N` is not nested across N. Cross-N ratio
tables are descriptive only — use `--task-ids` for nested arcs.

## Related rules

- `_shared/apxm-evaluation-rules.md`
- `_shared/apxm-preregistration-rules.md`
