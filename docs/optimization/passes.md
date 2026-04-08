---
title: "Optimization Passes"
description: "Compiler optimization passes that reduce latency, eliminate redundancy, and improve agent workflow efficiency."
---

# Optimization Passes

The APXM compiler runs a configurable sequence of optimization passes over the AIS MLIR representation. This page is the authoritative reference for every pass: what it does, which optimization levels include it, and how passes compose into the full pipeline. For a user-facing guide to choosing optimization levels and targets, see the [Optimization Guide](overview.md).

The pipeline source of truth is `crates/apxm-compiler/src/passes/pipeline.rs`. For the broader four-stage compilation architecture, see [Compilation Pipeline](../implementation/compiler/overview.md).

---

## AIS-Specific Passes

### normalize

**Levels:** O1, O2, O3

Normalizes graph structure into canonical form. Ensures consistent node ordering, resolves implicit attributes, and standardizes edge representations so that downstream passes operate on a uniform IR.

### build-prompt

**Levels:** O1, O2, O3

Assembles prompt templates for ASK and THINK operations from their constituent parts (system instructions, context references, user templates). Resolves template interpolation so later passes see fully-formed prompt strings.

### fuse-ask-ops

**Levels:** O1, O2, O3

Identifies producer-consumer chains of ASK operations where one ASK's output feeds directly into the next ASK's prompt. Fuses them into a single combined call, reducing API round-trips. The pass is conservative: it will not fuse across TRY_CATCH boundaries, across FENCE barriers, or when the intermediate result has multiple consumers.

### condense-ops

**Levels:** O2, O3

Condenses consecutive QMEM/UMEM operations targeting the same memory space into batched single-node operations, reducing the number of memory-access round-trips.

### scheduling

**Levels:** O1, O2, O3

Analyzes data dependencies to determine which operations can execute in parallel. Inserts synchronization points and computes the critical path for the runtime scheduler.

---

## Analysis Passes

### unconsumed-value-warning

**Levels:** O1, O2, O3

Emits warnings for operations whose output values are never consumed by any downstream operation. Helps authors identify dead branches or missing connections in their workflows.

---

## Standard MLIR Passes

These are built-in MLIR infrastructure passes adapted to the AIS dialect.

### canonicalizer

**Levels:** O1, O2, O3

MLIR's built-in canonicalization pass. Rewrites operations into standard forms -- normalizing branch conditions, collapsing single-input MERGE/WAIT_ALL into direct edges, removing empty TRY_CATCH wrappers. Reduces the pattern space for subsequent passes.

### cse

**Levels:** O1, O2, O3 (skipped when `--no-cse-llm` is set)

Common Subexpression Elimination. Identifies operations with identical opcodes and identical inputs, replacing duplicates with a single operation whose output is shared. For LLM operations, this treats same-prompt + same-context as equivalent (sound at temperature 0). Use `--no-cse-llm` to disable CSE for non-deterministic workflows.

### symbol-dce

**Levels:** O1, O2, O3

Symbol Dead Code Elimination. Removes unused symbol definitions (functions, globals) from the module. Runs last in the pipeline to clean up anything made dead by prior passes.

---

## O2-Only Passes

These passes are added at O2 and above, providing deeper optimization beyond the O1 baseline.

### template-specialization

**Levels:** O2, O3

Specializes generic prompt templates for specific operation contexts, producing tighter prompts that reduce token usage and improve model focus.

### schema-narrowing

**Levels:** O2, O3

Narrows parameter schemas to their tightest valid types based on data-flow analysis. Reduces the surface area for runtime type-checking and enables the model to produce more constrained outputs.

### dead-context-elimination

**Levels:** O2, O3

Removes context values that are threaded through the graph but never actually read. Unlike symbol-dce (which operates on top-level symbols), this targets intermediate context bindings within workflows.

---

## Graph-Level Passes

These passes operate on the `ApxmGraph` representation *before* MLIR lowering (at O2 and above). They annotate graph nodes with hint attributes that are then visible in the generated MLIR.

### prompt_caching

**Levels:** O2, O3 (pre-MLIR)

Detects shared prompt prefixes across ASK/THINK operations and marks them for caching, reducing redundant token processing at runtime.

### memoization_hints

**Levels:** O2, O3 (pre-MLIR)

Identifies deterministic operations (side-effect-free, fixed inputs) and annotates them for cross-run result caching. The runtime can skip re-execution when a cache hit occurs.

---

## Pipeline Composition

The pass ordering for each optimization level is defined in `build_pass_list()`. The [Optimization Guide](overview.md) describes what each level is designed for and when to choose it.

| Level | Pass sequence |
|-------|---------------|
| **O0** | *(none)* |
| **O1** | `normalize`, `build-prompt`, `unconsumed-value-warning`, `scheduling`, `fuse-ask-ops`, `canonicalizer`, `cse`\*, `symbol-dce` |
| **O2** | `normalize`, `build-prompt`, `template-specialization`, `unconsumed-value-warning`, `schema-narrowing`, `scheduling`, `fuse-ask-ops`, `condense-ops`, `dead-context-elimination`, `canonicalizer`, `cse`\*, `symbol-dce` |
| **O3** | Preamble: `normalize`, `build-prompt`, `unconsumed-value-warning`. Then up to 10 convergence iterations of: `template-specialization`, `schema-narrowing`, `scheduling`, `fuse-ask-ops`, `condense-ops`, `dead-context-elimination`, `canonicalizer`, `cse`\*, `symbol-dce` |

\* Skipped when `--no-cse-llm` is set.

At O2 and above, graph-level passes (`prompt_caching`, `memoization_hints`) run before the MLIR pipeline begins.

---

## Configuration

```bash
# Standard optimization (O2 is the default)
apxm compile workflow.apxm -O2 -o workflow.apxmobj

# No optimization
apxm compile workflow.apxm -O0 -o workflow.apxmobj

# Skip CSE for non-deterministic workflows
apxm compile workflow.apxm --no-cse-llm -o workflow.apxmobj

# Per-pass timing diagnostics
apxm compile workflow.apxm --emit-diagnostics diag.json
```

The `--emit-diagnostics` flag produces a JSON report with per-pass metrics: name, duration, ops before/after, and ops delta. See [Metrics Infrastructure](../benchmarks/methodology/metrics-infrastructure.md) for the full measurement framework.

---

## References

- [Optimization Guide](overview.md) -- optimization levels, targets, and best practices
- [Compilation Pipeline](../implementation/compiler/overview.md) -- the four-stage pipeline these passes plug into
- [Metrics Infrastructure](../benchmarks/methodology/metrics-infrastructure.md) -- how pass-level metrics are collected and reported
- [Benchmark Results](../benchmarks/results/2026-04-08.md) -- measured impact of O0 vs O2 on real workloads

### Academic

1. C. Lattner et al., "MLIR: Scaling Compiler Infrastructure for Domain Specific Computation," in *Proc. CGO '21*, IEEE, 2021. DOI: [10.1109/CGO51591.2021.9370308](https://doi.org/10.1109/CGO51591.2021.9370308)

2. J. Cocke, "Global Common Subexpression Elimination," in *Proc. Symposium on Compiler Optimization*, ACM, 1970. DOI: [10.1145/800028.808480](https://doi.org/10.1145/800028.808480)
