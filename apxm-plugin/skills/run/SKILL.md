---
name: run
description: Execute a pre-compiled .apxmobj artifact
user-invocable: true
---

# Run

Executes a pre-compiled `.apxmobj` artifact without re-compilation. This is the production execution path — you compile once with `dekk apxm compile`, then run the artifact as many times as needed. Skipping compilation makes startup faster and ensures you're running exactly the same optimized workflow every time.

The artifact is integrity-checked on load (BLAKE3 hash verification) to detect corruption from disk errors or incomplete writes.

## Commands

```bash
dekk apxm run workflow.apxmobj                         # execute the artifact
dekk apxm run workflow.apxmobj --emit-metrics m.json   # execute and write runtime statistics
dekk apxm run workflow.apxmobj -- "query" "context"    # pass arguments to the entry flow
```

## Artifact Loading

When the artifact is loaded:

1. The 52-byte header is parsed (magic bytes, version, payload length)
2. The BLAKE3 hash of the payload is computed and compared against the stored hash — a mismatch means the file is corrupted
3. The bincode payload is deserialized into DAGs, metadata, and sections
4. Agents and sub-flows are reconstructed from the artifact's DAG metadata
5. The `@entry` flow is located — every artifact must have exactly one

Arguments are positional and must match the entry flow's parameter count. All arguments are passed as strings.

## Related Commands

- `dekk apxm compile graph.apxm` — create the `.apxmobj` artifact that `run` consumes
- `dekk apxm execute graph.apxm` — compile and run in one step (no artifact file)
- `dekk apxm decompile workflow.apxmobj` — reverse-map an artifact back to graph JSON for inspection

## When to Use

- Production execution of pre-compiled workflows
- Running the same workflow repeatedly without recompilation overhead
- Distributing compiled artifacts to environments without the MLIR compiler
- Benchmarking execution time separately from compilation time
