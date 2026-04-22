# ablation

Phase-B compiler-pass ablation harness. See
`docs/superpowers/plans/2026-04-22-apxm-compiler-ablation.md`.

## Run

From the parent of the `ablation` package (so `-m ablation` resolves):

```
cd tools && python3 -m ablation
```

The harness compiles each stress graph in
`examples/python/_benchmarks/*.air` once at `-O 2` to establish a baseline,
then re-compiles each graph with each MLIR-side pass disabled
(`--disable-pass <name>`). It prints a Markdown delta table of
`ops_after` per (pass, graph) cell and exits non-zero when:

- a pass fires zero times across the entire corpus (dead pass), OR
- disabling a pass regresses `ops_after` by more than 5% on any graph.

`fired` is approximated as `fired_count > 0 OR ops_delta != 0` until Task 7
in the Phase B plan wires per-pass `_fired_count` IntegerAttrs into every
C++ transform, after which `fired_count` becomes authoritative.

## Conflict matrix

```
cd tools && python3 -m ablation.conflict_matrix
```

Runs the two known order-sensitive pass pairs in both orderings and prints
any cell where the result diverges (different `final_ops` or different
`fired_passes`).
