# APXM

APXM is a **graph-aware dispatch and scheduling layer for vLLM**. It splits the
work across an AMD-aligned CPU/GPU boundary: planning, validation, compilation,
and analysis stay on CPU; inference runs on GPU through a vLLM fork that
accepts dispatch hints. The public surface is an MLIR dialect (AIS), a Rust
runtime, and a vLLM fork at `apxm-project/vllm`.

APXM sits underneath the frameworks and orchestrators that call vLLM — a
typed IR plus a runtime that lets higher-level systems express *what* they
want dispatched and lets the platform decide *how*.

## What is in this repo

- **AIS dialect** (`crates/core/`) — the public IR contract. TableGen-defined
  ops; `apxm-core` is the only crate that defines AIS ops.
- **Compiler** (`crates/compiler/`) — MLIR pass pipeline + Python frontend
  (decorator DSL → canonical AIR).
- **Runtime** (`crates/runtime/`) — executor, handlers, backend adapters
  (LLM, local, tool); `apxm-backends` holds the vLLM-fork glue.
- **Tools** (`crates/tools/`) — `apxm-cli`, `apxm-server` (HTTP + MCP), the
  one inventory of installed skills that every client reads from.
- **`external/vllm`** — git submodule, vLLM fork on branch
  `apxm-rebase-v0.21.0`. Pinned to `apxm-project/vllm`.

## Getting started

Use [Dekk](https://github.com/randreshg/dekk) as the authority CLI. Every
build, test, and run goes through it so the env contract, target dir, and
process accounting stay consistent.

```bash
git clone https://github.com/apxm-project/apxm
cd apxm
git submodule update --init --recursive      # external/vllm fork
dekk apxm install --no-interactive
dekk apxm doctor                              # verify environment
dekk apxm build                               # release build
```

`dekk apxm doctor` is the first command of every session. It prints the
resolved environment (MLIR/LLVM 22, conda env, `CARGO_TARGET_DIR`, vLLM image
store, HF cache, service registry) and refuses to continue if anything is
misaligned.

## Companion repos

The APXM org follows the lineage: PXM theory led to APXM core; APXM then
produced children for evaluation, reusable libraries, the always-on supervisor,
the GUI, and the graph-aware vLLM fork. Each child owns its own docs, rules,
and release surface:

- **[`apxm-project/apxm-eval`](https://github.com/apxm-project/apxm-eval)** —
  preregistrations, benchmarks, claim cards, paper drafts. Any quality or
  performance claim against this runtime is preregistered and reproduced here.
- **[`apxm-project/apxm-libs`](https://github.com/apxm-project/apxm-libs)** —
  compiled-skill library loaded by `apxm-server` via `APXM_SKILLS_PATH`. Each
  pack ships a hash-pinned `.apxmobj` artifact plus its manifest.
- **[`apxm-project/apxm-gui`](https://github.com/apxm-project/apxm-gui)** —
  axum backend + React frontend served as a standalone binary. Install
  separately; `dekk apxm gui` shells out to it when it is on `PATH`.
- **[`apxm-project/apxm-os`](https://github.com/apxm-project/apxm-os)** —
  always-on supervisor for long-lived APXM agents and their manifests.
- **[`apxm-project/vllm`](https://github.com/apxm-project/vllm)** — the APXM
  fork of vLLM that accepts dispatch hints. Vendored as the
  `external/vllm` submodule on branch `apxm-rebase-v0.21.0`.

## Documentation

- [`docs/README.md`](docs/README.md) — full docs index.
- [`docs/pxm/readme.md`](docs/pxm/readme.md) — PXM theory and APXM's origin
  story.
- [`docs/compiler/pipeline.md`](docs/compiler/pipeline.md) — compiler pass
  pipeline.
- [`docs/backends/vllm.md`](docs/backends/vllm.md) — APXM/vLLM contract.
- [`docs/backends/storage-layout.md`](docs/backends/storage-layout.md) —
  where APXM puts large files (HF cache, image store, artifacts).
- [`apxm-project/apxm-libs`](https://github.com/apxm-project/apxm-libs) —
  home for skill-library design and pack documentation.
- Run `dekk apxm --help` and `dekk apxm ops list` for live CLI and AIS
  references.

## Contributing and license

See [`CONTRIBUTING.md`](CONTRIBUTING.md) for the clone flow, the lifecycle
workflow that every non-trivial session routes through, and the commit-message
rules. Released under the [MIT License](LICENSE); the bundled vLLM fork at
[`external/vllm`](external/vllm) is Apache-2.0.

For coding agents (Claude Code, Codex CLI, Cursor, Aider, Gemini): read
[`AGENTS.md`](AGENTS.md) (or [`CLAUDE.md`](CLAUDE.md)) before doing any work.
The 6-skill lifecycle — `/apxm-org:apxm-context` → `apxm-plan` →
`apxm-execute-plan` → `apxm-simplify` → `apxm-finish` → `apxm-commit` — is
the project-wide pattern.
