# apxm-compiler

MLIR-based compiler for the APXM graph contract and AIS MLIR dialect.

## Overview

`apxm-compiler` compiles APXM workflows through an MLIR-based pipeline. Rust,
Python, and TypeScript frontends converge on `FrontendGraph`; this crate
validates that DTO and owns the sole AIR printer. Canonical AIR then lowers to
the AIS MLIR dialect, runs the resolved pass pipeline, and produces executable
`.apxmobj` artifacts.

```
Rust / Python / TypeScript authoring
     ↓ FrontendGraph
Rust AirModule / AirProgram validator + printer
     ↓ canonical .air text
     ↓ Module::parse()
MLIR in memory
     ↓ PassManager::run() [configured pass list via FFI]
Optimized MLIR
     ↓ apxm_codegen_emit_artifact()
ExecutionDag
     ↓ Artifact::to_bytes()
.apxmobj (binary)
```

The crate has two layers:
- **Rust** (`src/`) -- FFI wrappers, `AirModule` builder, pass management, codegen
- **C++/MLIR** (`mlir/`) -- AIS dialect definition, optimization passes, artifact emitter

## Module Structure

| Module | Description |
|--------|-------------|
| `air_builder/` | `AirModule`, `AirNode`, `AirEdge`, `AirParam` graph builder and emitter |
| `api/context` | `Context` wrapping MLIR context and dialect registration |
| `api/module` | `Module` for parsed MLIR modules |
| `api/pipeline` | `Pipeline` orchestrating parse, optimize, and codegen stages |
| `codegen/artifact` | Artifact generation from optimized MLIR |
| `codegen/` | Code generation utilities |
| `passes/pipeline` | Pass pipeline configuration and ordering |
| `passes/manager` | `PassManager` for pass registration and execution |
| `passes/profile` | `ExecutionProfile` / `NodeProfile` for profiling |
| `passes/metrics` | `PassMetrics` / `PipelineDiagnostics` for compilation statistics |
| `passes/registry` | Pass discovery (`list_passes`, `find_pass`, `get_pass_count`) |
| `passes/validate_model_allowlist` | Model allowlist validation |
| `token_estimate` | Token count estimation for prompt budgeting |
| `ffi/` | Bindgen-generated FFI to the C++ MLIR library |

## MLIR Passes

The compiler exposes these MLIR passes. The default O-level pipelines only use
the production-safe subset; semantic rewrites remain explicit until their typed
contracts are enforced.

- `normalize` -- canonical form normalization
- `build-prompt` -- materializes templates only after explicit positional prompt roles establish input channels
- `template-specialization` -- folds constant `user` prompt inputs without collapsing protected channels
- `dead-context-elimination` -- removes only unused `user` context and retains protected channels
- `scheduling` -- emits backend-agnostic scheduling metadata
- `shared-prefix-analysis` -- annotates existing prefix-reuse opportunities
- `assign-priority` -- priority annotation for scheduling
- `dspy-optimize` -- fail-closed offline evaluation pass; public production compiler paths reject it
- `unconsumed-value-warning` -- opt-in diagnostic for unused produced values
- `fuse-ask-ops` -- explicit-only ASK fusion experiment; not a production claim
- `prompt-canonicalization` -- explicit-only prefix-cache layout experiment
- `condense-ops` -- explicit-only memory batching experiment
- `schema-narrowing` -- explicit-only field narrowing experiment
- canonicalizer and symbol-DCE (standard MLIR passes); generic CSE remains explicit-only

## MLIR Dialect

The AIS MLIR dialect is defined in `mlir/include/ais/Dialect/AIS/IR/`:

| File | Defines |
|------|---------|
| `AISDialect.td` | Dialect registration (`ais` namespace) |
| `AISOps.td` | AIS operations as MLIR ops |
| `AISTypes.td` | Type system (`!ais.token`, `!ais.handle`) |
| `AISAttributes.td` | Operation attributes |

TableGen is generated from the AIS authoring/codegen source:
``` 
crates/machine/ais/src/operations/definitions.rs  →  tablegen.rs  →  AISOps.td
crates/machine/ais/src/attrs.rs                   →                  AISAttributes.td
```

### C++ Implementation

| File | Purpose |
|------|---------|
| `mlir/lib/Dialect/AIS/IR/AISDialect.cpp` | Dialect registration |
| `mlir/lib/Dialect/AIS/IR/AISOps.cpp` | Operation definitions and verifiers |
| `mlir/lib/Dialect/AIS/IR/AISTypes.cpp` | Type system implementation |

### C API (FFI Bridge)

| File | Purpose |
|------|---------|
| `mlir/lib/CAPI/Module.cpp` | Parse / verify / to_string |
| `mlir/lib/CAPI/PassManager.cpp` | Run passes |
| `mlir/lib/CAPI/CodeGen.cpp` | Emit artifact |

Rust calls these via `src/ffi/raw.rs` (bindgen-generated).

## Codegen

Lowers optimized MLIR into an `ExecutionDag`, then serializes as `.apxmobj`:
```
Optimized MLIR  →  apxm_codegen_emit_artifact() [FFI]  →  ExecutionDag  →  Artifact bytes
```

`ArtifactEmitter.cpp` walks the MLIR and produces the DAG. `codegen/artifact.rs` handles wire format v3 serialization. `apxm-artifact` defines the binary container format.

## Prompt input roles

Every ASK, THINK, or REASON node with prompt inputs carries an explicit
`input_roles` array positional with `input_names` and context operands. The
allowed roles are `user`, `system`, `dependency_only`, `tool_context`, and
`control`. Compiler transformations do not infer roles from input names;
they preserve protected roles and only specialize or prune `user` inputs.
Frontend AIR validation rejects a context-bearing LLM node without this
contract, and artifact validation rejects malformed role arrays before an
artifact is published.

DSPy prompt optimization is not available from public production compiler
entry points. Offline evaluation owns the complete request, backend evidence,
and optimizer response; missing or incomplete DSPy input fails closed.

## Key Exports

- `AirModule` / `AirNode` / `AirEdge` / `AirParam` -- graph builder types
- `Context` -- MLIR context wrapper
- `Module` -- parsed MLIR module
- `Pipeline` -- configurable compilation pipeline
- `PassManager` -- pass registration and execution
- `ExecutionProfile` / `NodeProfile` -- profiling data
- `PipelineDiagnostics` / `PassMetrics` -- compilation statistics

## Dependencies

| Crate | Purpose |
|-------|---------|
| apxm-ais | Build-time authoring/codegen source for TableGen and pass descriptors |
| apxm-core | Shared downstream graph contract, error types, compiler options, constants |
| apxm-artifact | Artifact serialization |

## Requirements

- LLVM 22 / MLIR 22 (managed by Dekk)
- CMake 3.20+
- C++17 compiler

## Building

```bash
dekk agents build
```
