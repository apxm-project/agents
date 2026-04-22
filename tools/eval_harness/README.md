# eval_harness

Phase-A evaluation utilities. See
`docs/superpowers/plans/2026-04-21-apxm-evaluation-framework.md`.

## trace_diff

```
python3 -m eval_harness.trace_diff a.json b.json
```

Exits 0 with `equivalent`, or 1 with `divergent_at_node:N`. The diff sorts
events by sorted-parent-deps to absorb independent reorderings; remaining
differences indicate a real divergence between the two traces.
