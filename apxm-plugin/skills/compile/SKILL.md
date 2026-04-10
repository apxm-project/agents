---
name: compile
description: Compile a graph JSON into an optimized .apxmobj artifact
user-invocable: true
---

# Compile

Takes an AIS source file (.ais) or a project directory and compiles it through the MLIR-based optimization pipeline into a `.apxmobj` binary artifact. This is the "compiler" half of APXM — it parses your workflow, lowers it to the AIS MLIR dialect, runs optimization passes (fusing LLM calls, eliminating dead code, scheduling for parallelism), and emits a deterministic, hash-verified binary.

The compiled artifact is self-contained: it includes all DAGs, sub-flows, parameter schemas, and metadata needed for execution. Artifacts are content-addressable — identical source graphs produce byte-identical artifacts.

## Commands

```bash
dekk apxm compile graph.apxm                           # compile with default settings (O1)
dekk apxm compile graph.apxm -o workflow.apxmobj       # specify output path
dekk apxm compile graph.apxm -O0                       # no optimizations (raw IR to artifact)
dekk apxm compile graph.apxm -O2                       # standard optimizations (fuse + specialize + narrow)
dekk apxm compile graph.apxm -O3                       # aggressive (iterate passes to fixed-point)
dekk apxm compile graph.apxm --emit-diagnostics d.json # write per-pass compilation statistics
dekk apxm compile graph.apxm --no-cse-llm              # skip CSE for LLM ops (use when temperature > 0)
dekk apxm compile myproject/                           # compile all graphs in a project directory
```

## Optimization Levels

- **O0** — No passes. Useful for debugging the raw IR or when you need a baseline.
- **O1** (default) — Normalize, build prompts, schedule for parallelism, fuse consecutive ASK chains (1.29x fewer API calls), CSE, dead-code elimination.
- **O2** — Everything in O1 plus template specialization, schema narrowing, dead context elimination, and operation condensing.
- **O3** — O2 passes iterated up to 10 times until no further changes (fixed-point convergence). Typically converges in 2-4 iterations.

## Compilation Pipeline

1. **Load** — Parse graph JSON (or discover and merge `.apxm` files from a directory)
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
