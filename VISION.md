# Vision — APXM abstract machine, compiler, and runtime

APXM is the abstract-machine substrate for typed agent programs. Rust, Python,
and TypeScript author the same compiler-owned `FrontendGraph`; the Rust
compiler validates and prints canonical AIR, lowers it through MLIR, and emits
portable `.apxmobj` artifacts for the runtime.

The project is grounded in PXM theory: make agentic work explicit as programs,
then give those programs compiler, runtime, scheduling, and evidence surfaces.
The AIS dialect, compiler, runtime, CLI, and capability contracts live here.
`apxm-server`, `apxm-os`, `apxm-auth`, and Studio remain separate workspace
owners; vLLM is one backend integration.

## Lineage

PXM theory led to APXM: an abstract machine where planning, validation,
compilation, and analysis stay in the typed program layer while inference is a
runtime backend concern. The IR, compiler, runtime, backend contracts, and PXM
origin story all live here.

The graph-aware vLLM fork is vendored as `external/vllm`; it is an optional
backend that can honor APXM dispatch hints end to end.

## What APXM Core Provides

1. **Typed authoring contract.** Rust, Python, and TypeScript converge on the
   compiler-owned `FrontendGraph`. Rust-owned AIS definitions generate the
   operation metadata and bindings each frontend consumes.
2. **Canonical compiler.** The Rust compiler is the sole AIR printer and
   validation boundary. AIR lowers through MLIR, where graph cleanup, scheduling
   metadata, diagnostics, and artifact generation happen deterministically.
3. **Runtime execution.** The Rust runtime executes compiled artifacts,
   dispatches node handlers, records metrics, and keeps generated artifacts
   under `.apxm/`.
4. **Backend contracts.** Backend adapters, including `external/vllm`, consume
   compiler-produced execution metadata without redefining graph semantics.
5. **Operational tooling.** `dekk agents` is the supported entry point for build,
   test, compile, execute, backend, vLLM, MCP, server, and process operations.

## Boundaries

APXM is not the workspace coordinator, HTTP server, OS host plane, Studio UI,
or secret store. Its job is to make graph execution, capability authority,
dispatch metadata, backend behavior, and evidence explicit enough that callers
can build reliable products on top.

## Where To Read Next

- [`docs/pxm/readme.md`](docs/pxm/readme.md) — PXM theory and origin story.
- [`docs/compiler/pipeline.md`](docs/compiler/pipeline.md) — compiler pass
  pipeline.
- [`docs/backends/vllm.md`](docs/backends/vllm.md) — APXM/vLLM backend
  contract.
- [`docs/backends/storage-layout.md`](docs/backends/storage-layout.md) — data,
  artifact, HF cache, image-store, and service-registry layout.
- [`docs/vllm-fork.md`](docs/vllm-fork.md) — fork integration
  contract and hard-stop conditions.
