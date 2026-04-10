---
name: decompile
description: Reverse-map a compiled .apxmobj artifact back to graph JSON
user-invocable: true
---

# Decompile

Takes a compiled `.apxmobj` artifact and reconstructs it back into a graph JSON file. This is the reverse of `compile` — useful for inspecting what the optimizer did to your graph, verifying that the compiled artifact matches expectations, or recovering a graph when you only have the artifact.

The decompiled graph shows the *optimized* structure: fused nodes, eliminated dead code, reordered operations. Comparing it to the original source graph reveals exactly what the compiler changed.

## Commands

```bash
dekk apxm decompile workflow.apxmobj                   # print graph JSON to stdout
dekk apxm decompile workflow.apxmobj -o recovered.apxm  # write to file
```

## Process

1. Loads the artifact and validates its BLAKE3 hash (detects corruption)
2. Extracts the first DAG (or `@entry` DAG)
3. Converts wire-format nodes and edges back to `ApxmGraph` JSON
4. Reconstructs parameters with type names
5. Emits pretty-printed JSON

## When to Use

- Debugging optimizer behavior: compile at O2, decompile, diff against source
- Recovering a graph from a compiled artifact when the source is lost
- Verifying artifact contents before distribution
- Understanding what `FuseAskOps` or `CSE` actually changed in your workflow
