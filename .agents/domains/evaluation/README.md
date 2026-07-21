# Domain — evaluation

Owner-local offline and observed prompt evaluation lives under `evaluation/`
in this repository.

## Surfaces

- `dekk agents offline-prompt-evaluation` — validate preregistered prompt-output bundles.
- `dekk agents observed-prompt-evaluation` — execute preregistered prompt arms and record evidence.
- `dekk agents test-offline-prompt-evaluation` — focused offline evaluation tests.
- `dekk agents test-observed-prompt-evaluation` — focused observed evaluation tests.

## Norm

Each bundle under `evaluation/` carries a `preregistration.json` that records
disjoint case ids, digests, and portable source provenance before execution.
Generated artifacts land at `.apxm/evaluation/<scenario>/runs/<UTC>/`.

## Paired-arm benchmarks

Cache-salt scoping must be **per `(arm, opt)`**, never per
`(iter, row)`. See `feedback_paired_arm_cache_salt_scoping`.

## tau2 reminder

`tau2 --seed S --num-tasks N` is not nested across N. Cross-N ratio
tables are descriptive only — use `--task-ids` for nested arcs.
