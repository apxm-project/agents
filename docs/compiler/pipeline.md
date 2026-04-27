# Compiler Optimization Pipeline

How the APXM compiler turns an authored agent graph into a runnable, deterministic
artifact. This document is conceptual — the live ordering and per-target tuning is in
`crates/compiler/apxm-compiler/src/passes/pipeline.rs::build_pass_list()`.

## What the Compiler Does

The compiler accepts AIR (the human-readable text IR), parses it into MLIR using the
`ais` dialect, runs an optimization pipeline composed of MLIR-level transforms and a
small set of Rust-side checks, and emits a `.apxmobj` artifact. The artifact is an
execution-ready DAG with stamped metadata that the runtime can dispatch without
re-deriving any decisions the compiler made.

There are two kinds of passes:

- **MLIR passes** are C++ transforms over the `ais` dialect. They do the work that
  benefits from MLIR's pattern matching, walker infrastructure, and dataflow
  analyses.
- **Rust-side passes** run around MLIR for work that needs Rust-side state, such
  as tool registry checks and driver-level model allowlist validation.

Both kinds appear in the pipeline list in execution order; the dispatcher routes
each name to the right backend.

## Pipeline Diagram

```
   AIR text  ──────►  parse + lower  ──────►  MLIR module (ais.* ops)
   (.air)                                          │
                                                   ▼
                                  ┌─────────────────────────────────┐
                                  │       Normalization phase       │
                                  │   normalize                     │
                                  │   build-prompt                  │
                                  └────────────────┬────────────────┘
                                                   ▼
                                  ┌─────────────────────────────────┐
                                  │       Optimization phase        │
                                  │   dspy-optimize (config-gated) │
                                  │   template-specialization       │
                                  │   dead-context-elimination      │
                                  │   canonicalizer                 │
                                  │   symbol-DCE                    │
                                  └────────────────┬────────────────┘
                                                   ▼
                                  ┌─────────────────────────────────┐
                                  │       Analysis phase            │
                                  │   scheduling                    │
                                  │   shared-prefix-analysis        │
                                  │   assign-priority               │
                                  │   unconsumed-value-warning*     │
                                  └────────────────┬────────────────┘
                                                   ▼
                                  ┌─────────────────────────────────┐
                                  │       Rust-side checks          │
                                  │   tool-binding   (validate)     │
                                  │   bind-tool-handlers (link)     │
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
`unconsumed-value-warning` is opt-in through `--warn` and is shown only to mark
where the diagnostic pass runs when requested.

## The Passes

### MLIR passes (current transforms in `crates/compiler/apxm-compiler/mlir/lib/Dialect/AIS/Transforms/`)

| Pass                          | Purpose                                                                        |
|-------------------------------|--------------------------------------------------------------------------------|
| `normalize`                   | Canonical form — dedup context, lowercase attribute names, sort sets           |
| `build-prompt`                | Fill empty prompt templates from upstream context where it can be inferred     |
| `dspy-optimize`               | Config-gated compiler prompt tuning; no-op without training data              |
| `unconsumed-value-warning`    | Diagnostic: warn on values produced but never read by a downstream node        |
| `scheduling`                  | Annotate nodes with tier, cost, and latency labels for the runtime scheduler   |
| `shared-prefix-analysis`      | Emit backend-agnostic prefix reuse and warmup eligibility metadata             |
| `fuse-ask-ops`                | Explicit-only ASK mutation experiment; keep out of default pipelines          |
| `assign-priority`             | Stamp critical-path priority on nodes to drive scheduler ordering              |
| `prompt-canonicalization`     | Explicit-only: reorder prompt fragments for backend prefix-cache experiments   |
| `template-specialization`     | Fold known constants into prompt templates                                     |
| `dead-context-elimination`    | Prune context entries no downstream prompt actually consumes                   |
| `schema-narrowing`            | Explicit-only: planned field narrowing; current pass is diagnostic/experimental |
| `condense-ops`                | Explicit-only: batch memory ops; not default until memory semantics are typed  |

(`Passes.cpp` exists alongside these but only registers them — it is not itself a
pass.)

### Rust-side passes (in `crates/compiler/apxm-compiler/src/passes/*.rs`)

| Pass                       | Purpose                                                                              |
|----------------------------|--------------------------------------------------------------------------------------|
| `tool-binding`             | Validate that every `INV_TOOL` resolves to a `REGISTER_CAPABILITY` it can dispatch   |
| `bind-tool-handlers`       | Copy `python_handler_id` from `REGISTER_CAPABILITY` onto each matching `INV_TOOL`    |
| `validate-model-allowlist` | Standalone driver-invoked check that every model id is in the configured allowlist   |

The MLIR passes flow through the MLIR pass manager. The Rust-side tool passes operate
on the `AirModule` and run as the dispatcher visits their names in the pipeline list.
`validate-model-allowlist` is invoked separately by the driver/CLI rather than as
part of `build_pass_list`.

Graph-aware backend metadata remains backend-agnostic in the artifact. MLIR passes
stamp generic graph attributes such as `priority`, `downstream_nodes`,
`shared_prefix_group`, `shared_prefix_est_tokens`, and `warmup_candidate`. The
runtime converts those attributes into typed `ApxmGraphHints`; concrete backends
then translate the typed structure to their own wire format.

## Optimization Levels

The pipeline is parameterized by an optimization level and an optimization target.
The actual sequence each `(level, target)` produces is defined by
`build_pass_list()`; the doc here only describes the intent.

- **O0** — passthrough. No optimization. Useful for debugging the lowering and for
  baseline performance comparisons.
- **O1** — basic. Normalization, prompt construction, config-gated prompt
  tuning, template specialization, dead-context-elimination, canonicalization,
  tool checks, symbol DCE, and priority metadata.
- **O2** — standard. Keeps the O1 cleanup and prompt-tuning path, then adds
  scheduling metadata, analysis-only shared-prefix hints, and priority metadata.
  This is the default safe optimization level for production artifacts.
- **O3** — aggressive but still contract-safe. Repeats template-specialization,
  dead-context-elimination, scheduling metadata, canonicalization, tool checks,
  and symbol DCE up to the configured iteration cap. Use it for diagnostics
  or measured production workloads that benefit from repeated cleanup.

Generic MLIR CSE is available through explicit pass lists, but it is not part of
the default O-levels until LLM purity/determinism is represented as a typed IR
contract.

## Config-Gated Prompt Tuning

`dspy-optimize` is injected into O1/O2/O3 immediately after `build-prompt` only
when compiler-owned prompt tuning config and training data are available. The
base O-level pass lists stay deterministic and side-effect free; the pipeline
adds DSPy after it sees an explicit prompt-tuning request. The compiler reads
that config from the APXM config file, normalizes training data into
`.apxm/cache/compiler/training`, writes optimizer cache artifacts under
`.apxm/cache/compiler/dspy`, and strips transient optimizer metadata before
artifact serialization.

The compiler-owned configuration is isolated under
`[compiler.optimization.prompt_tuning]`. The LLM used for prompt tuning is
declared in `[compiler.optimization.prompt_tuning.backend]`; the compiler does
not inspect runtime `[chat]` routing or `[[backends]]` registrations.

## Explicit-Only Passes

The following passes remain implemented and can be invoked with `--pass-list`
for controlled experiments, but they are not part of O1/O2/O3 defaults:

- `fuse-ask-ops`: explicit-only experiment that mutates ASK chains. It
  is not a production optimization claim; producer-consumer ASK opportunities
  should be reported as analysis until typed quality and request-semantics
  contracts exist.
- `prompt-canonicalization`: useful for backend prefix-cache experiments, but
  prompt layout rewrites need an explicit backend/graph-hint contract.
- `schema-narrowing`: current implementation is not field-use schema narrowing.
- `condense-ops`: memory batching needs typed memory-store semantics.

## Where to Read More

- Conceptual layer: [`pxm/compute.md`](../pxm/compute.md) — what "compute" means
  for an agent program, and how APXM's choice of MLIR fits the model.
- AIS surface: [`pxm/ais.md`](../pxm/ais.md) — the operation set the passes are
  rewriting. For the live op list, run `dekk apxm ops list`.
- Live source: `crates/compiler/apxm-compiler/src/passes/pipeline.rs` is the
  ground truth for ordering. The pass implementations live in the per-pass files
  next to it (Rust) and under `mlir/lib/Dialect/AIS/Transforms/` (C++).
