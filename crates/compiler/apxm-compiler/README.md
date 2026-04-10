# apxm-compiler

MLIR-based compiler for the Agent Instruction Set (AIS) dialect.

## Overview

`apxm-compiler` compiles APXM workflows through an MLIR-based pipeline. It parses `.air` (MLIR text) or JSON graph inputs, builds an intermediate `AirModule` representation, lowers to the AIS MLIR dialect, applies optimization passes, and generates executable `.apxmobj` artifacts.

```
Python (@compile)
     ↓ captures graph
ApxmGraph (Python IR)
     ↓ AirModule.to_air()
.air text (MLIR)
     ↓ Module::parse()
MLIR in memory
     ↓ PassManager::run() [15 passes via FFI]
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

The C++ pipeline applies these optimization passes:

- `normalize-agent-graph` -- canonical form normalization
- `build-prompt` -- prompt template materialization
- `fuse-ask-ops` -- fuses adjacent ASK/THINK/REASON operations
- `assign-priority` -- priority annotation for scheduling
- `dead-context-elimination` -- removes unused context propagation
- `prompt-canonicalization` -- prefix deduplication for KV-cache sharing
- `condense-ops` -- merges redundant operations
- `schema-narrowing` -- tightens output schemas
- CSE, canonicalizer, symbol-DCE (standard MLIR passes)

## MLIR Dialect

The AIS MLIR dialect is defined in `mlir/include/ais/Dialect/AIS/IR/`:

| File | Defines |
|------|---------|
| `AISDialect.td` | Dialect registration (`ais` namespace) |
| `AISOps.td` | AIS operations as MLIR ops |
| `AISTypes.td` | Type system (`!ais.token`, `!ais.handle`) |
| `AISAttributes.td` | Operation attributes |

TableGen is generated from Core definitions:
```
apxm-ais/src/operations/definitions.rs  →  tablegen.rs  →  AISOps.td
apxm-ais/src/attrs.rs                   →                  AISAttributes.td
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
| apxm-ais | Operation definitions and TableGen source |
| apxm-core | Error types, compiler options, shared constants |
| apxm-artifact | Artifact serialization |

## Requirements

- LLVM 21 / MLIR 21 (via conda: `environment.yaml`)
- CMake 3.20+
- C++17 compiler

## Building

```bash
dekk apxm build
```
