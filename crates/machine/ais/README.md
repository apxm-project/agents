# apxm-ais

`apxm-ais` is the sole Rust-owned source of the closed AIS operation families.
Every compiler, frontend, and runtime consumer uses this catalogue.

The public semantic family contains exactly five operations:

1. `model.call`
2. `capability.invoke`
3. `program.new`
4. `program.invoke`
5. `await.event`

The separate structural family contains compiler-emitted functions, regions,
blocks, values, branches, loops, task joins, try/catch, return, and yield.
Structural operations are not a raw authoring API; `ais.loop` is not a sixth
semantic operation.

The source of truth is
[`src/operations/definitions.rs`](src/operations/definitions.rs). Generated
TableGen and frontend files are outputs and must not be edited to hide source
drift.

```bash
dekk agents ops list
dekk agents check-frontend-codegen
```

After changing an AIS definition, run `dekk agents build-dialect` and then
`dekk agents codegen`.
