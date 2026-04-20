# Compiler Optimization Pipeline

How the APXM compiler turns an authored agent graph into a runnable, deterministic
artifact. This document is conceptual — the live ordering and per-target tuning is in
`crates/compiler/apxm-compiler/src/passes/pipeline.rs::build_pass_list()`.

## What the Compiler Does

The compiler accepts AIR (the human-readable text IR), parses it into MLIR using the
`ais` dialect, runs an optimization pipeline composed of MLIR-level transforms and a
few Rust-side passes, and emits a `.apxmobj` artifact. The artifact is an
execution-ready DAG with stamped metadata that the runtime can dispatch without
re-deriving any decisions the compiler made.

There are two kinds of passes:

- **MLIR passes** are C++ transforms over the `ais` dialect. They do the work that
  benefits from MLIR's pattern matching, walker infrastructure, and dataflow
  analyses.
- **Rust-side passes** run on the `AirModule` before MLIR parsing or as post-MLIR
  bookkeeping. They handle work that needs Rust-side state (the tool registry, the
  vLLM hint table, the model allowlist) which would be awkward to thread through
  MLIR pass options.

Both kinds appear in the pipeline list in execution order; the dispatcher routes
each name to the right backend.

## Pipeline Diagram

```
   AIR text  ──────►  parse + lower  ──────►  MLIR module (ais.* ops)
   (.air)                                          │
                                                   ▼
                                  ┌─────────────────────────────────┐
                                  │       Normalization phase       │
                                  │   normalize-agent-graph         │
                                  │   build-prompt                  │
                                  │   dspy-optimize                 │
                                  └────────────────┬────────────────┘
                                                   ▼
                                  ┌─────────────────────────────────┐
                                  │       Optimization phase        │
                                  │   fuse-ask-ops                  │
                                  │   dead-context-elimination      │
                                  │   prompt-canonicalization       │
                                  │   template-specialization       │
                                  │   schema-narrowing              │
                                  │   condense-ops                  │
                                  └────────────────┬────────────────┘
                                                   ▼
                                  ┌─────────────────────────────────┐
                                  │       Analysis phase            │
                                  │   capability-scheduling         │
                                  │   assign-priority               │
                                  │   unconsumed-value-warning      │
                                  │   canonicalizer / CSE / sym-DCE │
                                  └────────────────┬────────────────┘
                                                   ▼
                                  ┌─────────────────────────────────┐
                                  │       Rust-side stamping        │
                                  │   tool-binding   (validate)     │
                                  │   bind-tool-handlers (link)     │
                                  │   vllm-hints     (annotate)     │
                                  └────────────────┬────────────────┘
                                                   ▼
                                       ArtifactEmitter
                                                   │
                                                   ▼
                                          .apxmobj artifact
                                          (loaded by the runtime)
```

The phases are conceptual groupings — the compiler does not declare them as
boundaries internally. Pass ordering inside a phase, and which passes are present at
which optimization level, is decided in `build_pass_list()`.

## The Passes

### MLIR passes (12 transforms in `crates/compiler/apxm-compiler/mlir/lib/Dialect/AIS/Transforms/`)

| Pass                          | Purpose                                                                        |
|-------------------------------|--------------------------------------------------------------------------------|
| `normalize-agent-graph`       | Canonical form — dedup context, lowercase attribute names, sort sets           |
| `build-prompt`                | Fill empty prompt templates from upstream context where it can be inferred     |
| `dspy-optimize`               | Apply ML-tuned prompt rewrites when a DSPy artifact is available (stub today)  |
| `unconsumed-value-warning`    | Diagnostic: warn on values produced but never read by a downstream node        |
| `capability-scheduling`       | Annotate nodes with tier, cost, and latency labels for the runtime scheduler   |
| `fuse-ask-ops`                | Combine adjacent LLM ASK calls that share context to eliminate round-trips     |
| `assign-priority`             | Stamp critical-path priority on nodes to drive scheduler ordering              |
| `prompt-canonicalization`     | Reorder prompt fragments so a common prefix can be reused by KV-cache          |
| `template-specialization`     | Fold known constants into prompt templates                                     |
| `dead-context-elimination`    | Prune context entries no downstream prompt actually consumes                   |
| `schema-narrowing`            | Drop output-schema fields no downstream node reads                             |
| `condense-ops`                | Batch sequences of memory ops (read/write/append) into a single op             |

(`Passes.cpp` exists alongside these but only registers them — it is not itself a
pass.)

### Rust-side passes (in `crates/compiler/apxm-compiler/src/passes/*.rs`)

| Pass                       | Purpose                                                                              |
|----------------------------|--------------------------------------------------------------------------------------|
| `tool-binding`             | Validate that every `INV_TOOL` resolves to a `REGISTER_CAPABILITY` it can dispatch   |
| `bind-tool-handlers`       | Copy `python_handler_id` from `REGISTER_CAPABILITY` onto each matching `INV_TOOL`    |
| `vllm-hints`               | Stamp `_vllm_*` hint attrs on LLM nodes so the runtime can route to vLLM correctly   |
| `validate-model-allowlist` | Standalone driver-invoked check that every model id is in the configured allowlist   |

The MLIR passes flow through the MLIR pass manager. The Rust-side passes operate on
the `AirModule` and run as the dispatcher visits their names in the pipeline list.
`validate-model-allowlist` is invoked separately by the driver/CLI rather than as
part of `build_pass_list`.

## Optimization Levels

The pipeline is parameterized by an optimization level and an optimization target.
The actual sequence each `(level, target)` produces is defined by
`build_pass_list()`; the doc here only describes the intent.

- **O0** — passthrough. No optimization. Useful for debugging the lowering and for
  baseline performance comparisons.
- **O1** — basic. Normalization, scheduling, fusion, canonicalization, CSE, DCE.
  Target-aware nudges (e.g. `Cost`/`Tokens` get an extra dead-context-elimination
  pass).
- **O2** — standard. Adds template specialization, dead-context-elimination,
  schema-narrowing, condense-ops. Within O2, the *target* (`Latency`, `Cost`,
  `Tokens`, `Parallelism`, `Balanced`) reorders passes to favor the chosen
  objective.
- **O3** — aggressive. O2 iterated to a fixed-point (capped iterations) so that
  each pass observes the others' results.

## Where to Read More

- Conceptual layer: [`pxm/compute.md`](../pxm/compute.md) — what "compute" means
  for an agent program, and how APXM's choice of MLIR fits the model.
- AIS surface: [`pxm/ais.md`](../pxm/ais.md) — the operation set the passes are
  rewriting. For the live op list, run `dekk apxm ops list`.
- Live source: `crates/compiler/apxm-compiler/src/passes/pipeline.rs` is the
  ground truth for ordering. The pass implementations live in the per-pass files
  next to it (Rust) and under `mlir/lib/Dialect/AIS/Transforms/` (C++).
