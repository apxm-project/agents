---
title: "Compilation Pipeline"
description: "Four-stage pipeline from canonical ApxmGraph input to optimized .apxmobj execution artifact."
---

# Compilation Pipeline

The A-PXM compiler transforms canonical `ApxmGraph` input into an optimized, portable execution artifact. The pipeline has four stages, each with well-defined input and output formats.

## Pipeline Overview

```
  Graph JSON ──► Parse ──► Lower to MLIR ──► Optimize ──► Emit ──► .apxmobj
                  (1)          (2)              (3)         (4)
```

## Stage 1: Parse

**Input:** Graph JSON (or AIS DSL / directory layout)
**Output:** Canonical `ApxmGraph`

Parses the frontend source into an `ApxmGraph`, then validates canonical constraints (node IDs, edge references, DAG structure, parameter integrity). Errors at this stage produce human-readable diagnostics before any MLIR work begins.

## Stage 2: Lower to MLIR

**Input:** `ApxmGraph`
**Output:** Unoptimized AIS MLIR dialect

Converts graph IR into AIS Dialect MLIR: resolves implicit types, constructs typed operations with custom verifiers, and wraps subgraphs (TRY_CATCH scopes, BRANCH targets) in MLIR regions.

At O2 and above, two **graph-level passes** run on the `ApxmGraph` *before* MLIR lowering so that the resulting hint attributes are visible in the generated IR:

- `prompt_caching` -- detects shared prompt prefixes across operations and marks them for caching
- `memoization_hints` -- identifies deterministic operations and annotates them for cross-run caching

## Stage 3: Optimize

**Input:** Unoptimized AIS MLIR
**Output:** Optimized AIS MLIR

The optimizer runs a configurable sequence of MLIR passes. The pass list depends on the optimization level:

| Level | Passes | Description |
|-------|--------|-------------|
| **O0** | *(none)* | No optimization; passthrough |
| **O1** | `normalize`, `build-prompt`, `unconsumed-value-warning`, `scheduling`, `fuse-ask-ops`, `canonicalizer`, `cse`\*, `symbol-dce` | Basic optimizations (8 passes) |
| **O2** | `normalize`, `build-prompt`, `template-specialization`, `unconsumed-value-warning`, `schema-narrowing`, `scheduling`, `fuse-ask-ops`, `condense-ops`, `dead-context-elimination`, `canonicalizer`, `cse`\*, `symbol-dce` | Standard optimizations (12 passes) |
| **O3** | Preamble (`normalize`, `build-prompt`, `unconsumed-value-warning`) then the O2 optimization core iterated up to 10 times for fixed-point convergence | Aggressive optimizations |

\* CSE is skipped when the `--no-cse-llm` flag is set (useful for non-zero temperature workflows).

See [Optimization Passes](../../optimization/passes.md) for detailed descriptions of each pass.

## Stage 4: Artifact Emit

**Input:** Optimized AIS MLIR
**Output:** `.apxmobj` binary artifact

The emitter serializes the optimized dataflow graph into a bincode-serialized `ArtifactPayload`:

1. **DAG serialization**: encode nodes, edges, and subgraphs into a compact binary representation.
2. **Metadata embedding**: attach AAM declarations, capability schemas, and compilation flags.
3. **Entry point registration**: mark top-level workflow entry points for the runtime loader.
4. **Version stamping**: embed the artifact format version for backwards compatibility.

See [Artifact Format](artifact-format.md) for the binary layout.

## Error Reporting

The compiler provides errors at the earliest possible stage:

- **Parse errors**: invalid graph shape, missing fields, malformed JSON
- **MLIR errors**: type mismatches, invalid operations, verifier failures
- **Optimization warnings**: dead code, unused capabilities, unconsumed values
- **Emit errors**: schema violations

Compile-time checking catches structural errors before any LLM call is made, before any tool is invoked, before any cost is incurred.

## CLI Usage

```bash
# Full pipeline: graph source to artifact
apxm compile workflow.apxm -o workflow.apxmobj

# No optimization
apxm compile workflow.apxm -o workflow.apxmobj -O0

# Aggressive optimization with convergence
apxm compile workflow.apxm -o workflow.apxmobj -O3

# Skip CSE for non-deterministic workflows
apxm compile workflow.apxm -o workflow.apxmobj --no-cse-llm

# Emit per-pass diagnostics
apxm compile workflow.apxm -o workflow.apxmobj --emit-diagnostics diag.json
```

---

## Related Documentation

- [Compiler Integration](../compiler-integration.md) -- how JSON, Rust, and Python frontends feed this pipeline
- [Optimization Passes](../../optimization/passes.md) -- detailed descriptions of every MLIR pass
- [Artifact Format](artifact-format.md) -- `.apxmobj` binary layout
- [Graph JSON Format](../../reference/graph-format.md) -- canonical graph schema consumed by Stage 1

## References

1. C. Lattner and V. Adve, "LLVM: A Compilation Framework for Lifelong Program Analysis & Transformation," in *Proc. CGO '04*, IEEE, 2004. DOI: [10.1109/CGO.2004.1281665](https://doi.org/10.1109/CGO.2004.1281665)

2. C. Lattner et al., "MLIR: Scaling Compiler Infrastructure for Domain Specific Computation," in *Proc. CGO '21*, IEEE, 2021. DOI: [10.1109/CGO51591.2021.9370308](https://doi.org/10.1109/CGO51591.2021.9370308)
