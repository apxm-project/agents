# Vision — graph-aware dispatch for vLLM

APXM is the core runtime and compiler substrate for graph-aware dispatch on
vLLM. It gives higher-level systems a typed graph IR, a deterministic compiler
pipeline, and a Rust executor that can preserve scheduling intent all the way
to a vLLM fork that understands APXM dispatch hints.

The project is grounded in PXM theory: make agentic work explicit as programs,
then give those programs real compiler, runtime, scheduling, and evidence
surfaces. In this repo, that theory becomes the AIS MLIR dialect, the APXM
runtime, the CLI/server, and the contract with the graph-aware vLLM fork.

## Lineage

PXM theory led to APXM core: a dispatch and scheduling layer for vLLM, with
an AMD-aligned CPU/GPU split where planning, validation, compilation, and
analysis stay on CPU while inference runs on GPU. The IR, compiler, runtime,
backend contracts, and the PXM origin story all live here.

The graph-aware vLLM fork is vendored as `external/vllm` (branch
`apxm-rebase-v0.21.0`); the runtime depends on it to honor dispatch hints
end to end.

## What APXM Core Provides

1. **Typed graph IR.** AIS is the public contract. Ops and attributes are
   defined in `apxm-core`; compiler passes, the Python frontend, runtime
   handlers, and the vLLM fork consume that contract.
2. **Compiler passes.** AIR lowers through the MLIR-backed APXM compiler, where
   graph cleanup, scheduling metadata, backend hints, diagnostics, and artifact
   generation happen deterministically.
3. **Runtime execution.** The Rust runtime executes compiled artifacts,
   dispatches node handlers, records metrics, and keeps generated artifacts
   under `.apxm/`.
4. **vLLM dispatch contract.** Backend adapters and the `external/vllm`
   submodule preserve APXM hints until they reach a vLLM server that can act on
   graph-aware scheduling metadata.
5. **Operational tooling.** `dekk apxm` is the supported entry point for build,
   test, compile, execute, backend, vLLM, MCP, server, and process operations.

## Boundaries

APXM core is not an agent framework, an LLM orchestrator, or a multi-agent
runtime. It sits below those systems. Its job is to make graph execution,
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
- [`docs/external-vllm-fork.md`](docs/external-vllm-fork.md) — fork integration
  contract and hard-stop conditions.
