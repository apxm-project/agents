# APXM — A Library System for Agent Skills

> Software scaled when code became libraries. Agents will scale only when skills do.

Every team encodes the same processes as different "skills" and rewrites them.
APXM compiles each skill into a typed AIR graph and a reusable `.apxmobj`
artifact — the way a function compiles to an object file. The same artifact
runs across deployments with reproducible sessions, enforced capabilities, and
compiler diagnostics that compound across every existing skill.

Under the hood: an MLIR-based compiler (the AIS dialect), a runtime with
scheduling, memory, tools, backend routing, and graph metrics, a Dekk-first
CLI, and `apxm-server` — one skill inventory that Codex, Claude Code, the GUI,
and `apxm-cli` all see through a single REST/MCP surface.

For the "why" read [VISION.md](VISION.md); for the formal model
("LLVM for agents") read [docs/pxm/readme.md](docs/pxm/readme.md); for
hands-on, keep reading.

## Quick start

```bash
git clone https://github.com/randreshg/apxm && cd apxm
git submodule update --init --recursive      # external/vllm fork
dekk apxm install --no-interactive
dekk apxm doctor                              # verify environment
dekk apxm execute examples/python/getting-started/hello.air
```

Stand up a vLLM model zoo:

```bash
export APXM_VLLM_HF_HOME=$HOME/.cache/huggingface-apxm-vllm
cp deploy/vllm/zoo.example.toml deploy/vllm/zoo.toml && $EDITOR deploy/vllm/zoo.toml
dekk apxm vllm zoo-cache-warm && dekk apxm vllm zoo-apply
```

`APXM_VLLM_HF_HOME` must point at a filesystem every Slurm compute node
can read at the same path; pick a different mount than `$HOME` when home
is space-constrained. See
[docs/backends/storage-layout.md](docs/backends/storage-layout.md) for
the full rules and the supported migration procedure.

Full walkthrough: [docs/backends/model-zoo-quickstart.md](docs/backends/model-zoo-quickstart.md).

Prerequisites: Python 3.10+, conda/mamba, Git. The installer handles Rust
nightly, CMake, and MLIR/LLVM 22 in a repo-local conda env.

## Project layout

```text
crates/
  core/         # AIS dialect, graph types, codegen authoring
  compiler/    # MLIR compiler + Python frontend
  orchestration/# driver, artifact, ACP glue
  runtime/     # LLM backends, credentials, execution engine
  tools/       # CLI, HTTP/MCP server, browser GUI
examples/python/ # getting-started, parallelism, optimization, benchmarks, demos
deploy/vllm/   # zoo.example.toml, run-vllm.sh, Dockerfile.apxm
docs/          # start at docs/README.md
```

## Documentation

- [VISION.md](VISION.md) + [docs/pxm/readme.md](docs/pxm/readme.md) — positioning + formal model
- [docs/README.md](docs/README.md) — full docs index
- [docs/backends/model-zoo-quickstart.md](docs/backends/model-zoo-quickstart.md) — vLLM zoo in 15 minutes
- [docs/backends/model-zoo.md](docs/backends/model-zoo.md) — zoo operator reference
- [docs/backends/vllm.md](docs/backends/vllm.md) — APXM/vLLM contract
- [docs/backends/storage-layout.md](docs/backends/storage-layout.md) — where APXM puts large files (HF cache, image store, artifacts)
- [docs/design/apxm-aware-codex-skill-libraries.md](docs/design/apxm-aware-codex-skill-libraries.md) — skill libraries design
- [docs/compiler/pipeline.md](docs/compiler/pipeline.md) — compiler pass pipeline
- Run `dekk apxm --help` and `dekk apxm ops list` for live CLI / AIS reference

## Contributing & license

See [CONTRIBUTING.md](CONTRIBUTING.md) for build, test, and PR conventions, and
[CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md) for community standards. For security
issues, follow [SECURITY.md](SECURITY.md). Released under the
[MIT License](LICENSE); the bundled vLLM fork at
[`external/vllm`](external/vllm) is Apache-2.0.

**For coding agents** (Claude Code, Codex CLI, Cursor, Aider, Gemini): read
[AGENTS.md](AGENTS.md) (or [CLAUDE.md](CLAUDE.md)) before doing any work, and
follow the 6-skill lifecycle (`apxm-context` → `apxm-plan` →
`apxm-execute-plan` → `apxm-simplify` → `apxm-finish` → `apxm-commit`). For
Codex, run `dekk apxm skills sync` to sync skills into `~/.codex/skills/`.
See [DOMAIN.md](DOMAIN.md) for a one-page orientation.
