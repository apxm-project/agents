---
name: decompile
description: Reverse-map a compiled .apxmobj artifact back to AIR
user-invocable: true
---

# Decompile

Takes a compiled `.apxmobj` artifact and reconstructs it back into AIR. This is the reverse of `compile` — useful for inspecting what the optimizer did to your graph, verifying that the compiled artifact matches expectations, or recovering a graph when you only have the artifact.

The decompiled graph shows the *optimized* structure: eliminated dead code, scheduling metadata, and typed graph hints. Comparing it to the original AIR source reveals exactly what the compiler changed.

## Commands

```bash
dekk apxm decompile workflow.apxmobj                   # print AIR to stdout
dekk apxm decompile workflow.apxmobj -o recovered.air  # write to file
```

## Process

1. Loads the artifact and validates its BLAKE3 hash (detects corruption)
2. Extracts the first DAG (or `@entry` DAG)
3. Converts wire-format nodes and edges back to AIR
4. Reconstructs parameters with type names
5. Emits readable AIR

## When to Use

- Debugging optimizer behavior: compile at O2, decompile, diff against source
- Recovering a graph from a compiled artifact when the source is lost
- Verifying artifact contents before distribution
- Understanding what DCE, scheduling, or graph-hint passes changed in your workflow
