---
name: compile
description: Compile a graph JSON into an optimized .apxmobj artifact
user-invocable: true
---

# Compile

Takes a Python graph, AIR/MLIR source, graph JSON, or project directory and compiles it through the MLIR-based optimization pipeline into a `.apxmobj` binary artifact. This is the "compiler" half of APXM: it parses your workflow, lowers it to the AIS MLIR dialect, runs production-safe optimization passes, and emits a deterministic, hash-verified binary.

The compiled artifact is self-contained: it includes all DAGs, sub-flows, parameter schemas, and metadata needed for execution. Artifacts are content-addressable — identical source graphs produce byte-identical artifacts.

## Commands

```bash
dekk apxm compile graph.air                           # compile with default settings (O2)
dekk apxm compile graph.air -o workflow.apxmobj       # specify output path
dekk apxm compile graph.air -O0                       # no optimizations (raw IR to artifact)
dekk apxm compile graph.air -O2                       # standard safe optimizations
dekk apxm compile graph.air -O3                       # aggressive (iterate passes to fixed-point)
dekk apxm compile graph.air --emit-diagnostics d.json # write per-pass compilation statistics
dekk apxm compile graph.air --pass-list normalize,build-prompt # explicit pass ablation
dekk apxm compile myproject/                           # compile all graphs in a project directory
```

## Optimization Levels

- **O0** — No passes. Useful for debugging the raw IR or when you need a baseline.
- **O1** — Normalize, build prompts, run config-gated DSPy prompt optimization, template specialization, dead-context elimination, tool binding, and safe cleanup.
- **O2** (default) — Everything in O1 plus scheduling metadata and shared-prefix analysis according to the selected optimization target.
- **O3** — O2 passes iterated up to 10 times until no further changes (fixed-point convergence). Typically converges in 2-4 iterations.

## Compilation Pipeline

1. **Load** — Parse graph JSON (or discover and merge `.air` files from a directory)
2. **Lower** — Convert `ApxmGraph` to AIS dialect MLIR with type inference and verifier attachment
3. **Parse** — Feed MLIR text through the MLIR parser
4. **Verify** — Run MLIR verifiers (type checking, latency budget validation, capability existence)
5. **Optimize** — Run configured pass pipeline (see optimization levels above)
6. **Emit** — Serialize optimized DAGs into bincode with sorted attributes, compute BLAKE3 hash, write 52-byte header + payload

## Artifact Format

Binary layout: 4-byte magic (`APXM`) + version (u32) + payload length (u64) + BLAKE3 hash (32 bytes) + flags (u32) + bincode payload. The hash is verified on every load to detect corruption.

## Diagnostics Output

With `--emit-diagnostics`, the compiler writes a JSON report including:
- Total compilation time and artifact generation time
- Per-pass metrics: name, duration, ops before/after, ops eliminated
- DAG statistics: total nodes, entry/exit nodes, edge count
- Pass summary: total passes run, initial vs final op count

## When to Use

- When you want to pre-compile a graph for repeated execution (use `dekk apxm run` on the artifact)
- When you need compilation diagnostics to understand optimizer behavior
- When building production artifacts with deterministic, verifiable output
- Use `dekk apxm execute` instead if you want to compile and run in one step
