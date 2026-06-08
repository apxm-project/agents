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

### System dependencies

- **bubblewrap (`bwrap`)** — required to confine tool execution (`EXC`, the
  `bash` and user-tool capabilities, and sandboxed ACP agents). Without it the
  registry falls back to the policy-only process backend and `OsLevel` requests
  fail closed. Install with `apt-get install bubblewrap` (or your distro
  equivalent).
  - On Ubuntu 23.10+ unprivileged user namespaces are restricted by AppArmor
    (`kernel.apparmor_restrict_unprivileged_userns=1`), which blocks `bwrap`
    unless it has a profile granting `userns`, or it is setuid. Create the
    profile and activate it:

    ```bash
    sudo tee /etc/apparmor.d/bwrap >/dev/null <<'EOF'
    abi <abi/4.0>,
    include <tunables/global>
    profile bwrap /usr/bin/bwrap flags=(unconfined) {
      userns,
      include if exists <local/bwrap>
    }
    EOF
    sudo apparmor_parser -r /etc/apparmor.d/bwrap   # activate now; persists across reboot
    ```

    Placing the file in `/etc/apparmor.d/` makes it persistent (loaded by
    `apparmor.service` at boot); `apparmor_parser -r` only activates it
    immediately without a reboot. `dekk apxm doctor` reports whether `bwrap` is
    available. See
    [`docs/integrations/sandbox-acp-seam.md`](docs/integrations/sandbox-acp-seam.md).

## Documentation

- [`docs/README.md`](docs/README.md) — full docs index.
- [`docs/pxm/readme.md`](docs/pxm/readme.md) — PXM theory and APXM's origin
  story.
- [`docs/agent-topology-boundary.md`](docs/agent-topology-boundary.md) —
  hard rule: agent hierarchy and reachability are policy outside the runtime.
- [`docs/compiler/pipeline.md`](docs/compiler/pipeline.md) — compiler pass
  pipeline.
- [`docs/backends/vllm.md`](docs/backends/vllm.md) — APXM/vLLM contract.
- [`docs/backends/storage-layout.md`](docs/backends/storage-layout.md) —
  where APXM puts large files (HF cache, image store, artifacts).
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
