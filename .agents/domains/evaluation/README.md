# Domain — evaluation

Benchmarks, preregistration, claim cards, paper.

## Skills

- **apxm-preregistration** — draft & commit a
  `docs/preregistrations/` entry before claim-bearing runs.
- **apxm-priority-lane-bench** — priority-lane benchmark workflow.
- **apxm-review-council-bench** — review-council benchmark workflow.
- **apxm-claim-evidence** — write-up + claim card after a run.
- **apxm-evaluation-artifacts** — artifact placement under `.apxm/`.

## Norm

Every claim-bearing run requires a **committed preregistration before
the run starts**. `apxm-finish` enforces this — it refuses to claim
done if the prereg commit timestamp is after the first artifact.

## Layout

- `docs/preregistrations/<UTC>-<descriptor>.md` — frozen, append-only.
- `docs/claims/<id>.md` — claim cards for paper-bound numbers.
- `docs/evaluation/<scenario>/<UTC>.md` — write-ups.
- `.apxm/evaluation/<scenario>/runs/<UTC>/` — raw artifacts.
- `docs/paper/` — paper drafts.
- `docs/plans/MASTER.md` — the 9 numbered sub-plans.

## Paired-arm benchmarks

Cache-salt scoping must be **per `(arm, opt)`**, never per
`(iter, row)`. See `feedback_paired_arm_cache_salt_scoping`.

## tau2 reminder

`tau2 --seed S --num-tasks N` is not nested across N. Cross-N ratio
tables are descriptive only — use `--task-ids` for nested arcs.

## Related rules

- `_shared/apxm-evaluation-rules.md`
- `_shared/apxm-preregistration-rules.md`
