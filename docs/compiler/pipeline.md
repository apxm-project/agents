# Compiler optimization pipeline — pre-canonical baseline

> **Migration evidence, not target compiler authority.** Direct AIR/
> `apxm canonical-air` and current ASK/cognition-op behavior below describe the
> pre-canonical
> compiler. FrontendGraph v1, AIR v1, and P3/P9 of the
> [normative composition/AIR plan](../agents/agent-program-composition-and-air-full-replacement-plan.md)
> replace this path without a prototype reader.

How the APXM compiler turns an authored agent graph into a runnable,
deterministic artifact. Rust, Python, and TypeScript author the shared
`FrontendGraph` DTO; the Rust compiler owns validation and AIR printing. This
document follows the executable plan resolved by
`passes::resolve_pipeline_plan()` and `Pipeline::resolved_plan()`.
The cross-compiler/runtime acceptance gates are defined in
[`workflow-optimization-roadmap.md`](workflow-optimization-roadmap.md).

## What the Compiler Does

The compiler accepts canonical AIR (the human-readable text IR), parses it into
MLIR using the `ais` dialect, runs an optimization pipeline composed of
MLIR-level transforms and Rust-side checks, and emits a `.apxmobj` artifact.
Python and TypeScript never format AIR themselves: they send `FrontendGraph` to
`apxm canonical-air`, which invokes the Rust validator and native bridge.

There are two kinds of passes:

- **MLIR passes** are C++ transforms over the `ais` dialect. They do the work that
  benefits from MLIR's pattern matching, walker infrastructure, and dataflow
  analyses.
- **Rust-side checks/finalizers** run around MLIR for work that needs Rust-side
  state or artifact-level data, such as tool registry checks and driver-level
  model allowlist validation.

The typed `PipelinePlan` controls the MLIR transform sequence and records each
stage's required, preserved, and invalidated analyses. Artifact finalization
then runs invariant checks for every optimization level so O0 and O2 artifacts
share the same executable runtime contract.

## Pipeline Diagram

```
Rust builder ─┐
Python DSL ──┼──► FrontendGraph ─► Rust AIR printer ─► AIR text
TypeScript ──┘                                      │
Direct .air ────────────────────────────────────────┘
                                                     ▼
                                             MLIR module (ais.* ops)
                                                     │
                                                   ▼
                                  ┌─────────────────────────────────┐
                                  │       Required lowering         │
                                  │   normalize                     │
                                  │   build-prompt                  │
                                  └────────────────┬────────────────┘
                                                   ▼
                                  ┌─────────────────────────────────┐
                                  │       Optimization phase        │
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
                                  │       Artifact finalization     │
                                  │   template contract validation  │
                                  │   tool checks + handler links   │
                                  │   optimization-summary.v1       │
                                  └────────────────┬────────────────┘
                                                   ▼
                                       ArtifactEmitter
                                                   │
                                                   ▼
                                          .apxmobj artifact
                                          (loaded by the runtime)
```

The phases are real typed boundaries. `PipelinePlan` distinguishes required
lowering, MLIR rewrite and analysis stages, artifact validation, artifact
finalization, and diagnostics. `build_pass_list()` remains a compatibility view
of MLIR-only stages; `resolve_pipeline_plan()` is the executable source of
truth. Mandatory artifact stages cannot be removed by explicit pass lists or
ordinary pass-disable flags.
`unconsumed-value-warning` is opt-in through `--warn` and is shown only to mark
where the diagnostic pass runs when requested.

## The Passes

### MLIR passes (current transforms in `crates/compiler/pipeline/mlir/lib/Dialect/AIS/Transforms/`)

| Pass                          | Purpose                                                                        |
|-------------------------------|--------------------------------------------------------------------------------|
| `normalize`                   | Canonical form: lowercase selected attrs and dedup unnamed context operands    |
| `build-prompt`                | Materialize LLM `template_str` / `input_names` runtime contracts               |
| `dspy-optimize`               | Explicit offline-evaluation-only prompt search; production compilation rejects it |
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

### Rust-side passes (in `crates/compiler/pipeline/src/passes/*.rs`)

| Pass                       | Purpose                                                                              |
|----------------------------|--------------------------------------------------------------------------------------|
| `capability-binding-check`       | Validate that every `capability.invoke` resolves to an admitted capability it can dispatch |
| `validate-model-allowlist` | Standalone driver-invoked check that every model id is in the configured allowlist   |

The MLIR passes flow through the MLIR pass manager. The Rust-side artifact checks
operate on the emitted `ExecutionDag` and run for every optimization level during
artifact generation. `validate-model-allowlist` is invoked separately by the
driver/CLI rather than as part of `build_pass_list`.

Graph-aware backend metadata remains backend-agnostic in the artifact. MLIR passes
stamp generic graph attributes such as `priority`, `downstream_nodes`,
`shared_prefix_group`, `shared_prefix_est_tokens`, and `warmup_candidate`. The
runtime converts those attributes into typed `ApxmGraphHints`; concrete backends
then translate the typed structure to their own wire format.

## Optimization Levels

The pipeline is parameterized by an optimization level and an optimization target.
The actual sequence each `(level, target)` produces is defined by
`build_pass_list()`; the doc here only describes the intent.

- **O0** — required normalization and executable lowering only. It runs
  `normalize` and `build-prompt`, then artifact finalization validates the same
  runtime contracts used by higher optimization levels. It does not run cleanup,
  scheduling, priority, DSPy, or graph rewrites.
- **O1** — basic safe cleanup. It keeps the O0 lowering path, then adds
  template specialization, dead-context-elimination, canonicalization, symbol
  DCE, and priority metadata.
- **O2** — standard. Keeps the O1 cleanup path, then adds scheduling metadata
  and analysis-only shared-prefix hints. This is the default safe optimization
  level for production artifacts.
- **O3** — aggressive but still contract-safe. Runs a bounded cleanup
  convergence group of template specialization, typed context cleanup,
  scheduling metadata, canonicalization, and symbol DCE. The group stops at a
  fixed point or reports its iteration limit; artifact finalization still runs
  once after MLIR lowering.

Generic MLIR CSE is available through explicit pass lists, but it is not part of
the default O-levels until LLM purity/determinism is represented as a typed IR
contract.

## Unsupported Prompt Tuning

`dspy-optimize` is unavailable in production compilation. Prompt search is
quality-changing and model-dependent, so the compiler rejects it instead of
silently changing prompts. Compilation does not make model calls, read
prompt-tuning credentials, or mutate prompts through DSPy.

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

Promotion into the default O-levels requires a typed compiler/runtime contract,
tests that prove the rewrite preserves graph behavior, and a pass-list change in
`build_pass_list()` rather than an ad hoc pass-manager registration.

## Where to Read More

- AIS surface: [`pxm/ais.md`](../pxm/ais.md), the operation set the passes are
  rewriting. For the live op list, run `dekk agents ops list`.
- Live source: `crates/compiler/pipeline/src/passes/pipeline.rs` is the
  ground truth for ordering. The pass implementations live in the per-pass files
  next to it (Rust) and under `mlir/lib/Dialect/AIS/Transforms/` (C++).
