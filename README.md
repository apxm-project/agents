# agents

`agents` is the APXM abstract-machine repo: AIS dialect, compiler, runtime,
capability contracts, context handling, permissions, orchestration, CLI, and
the profile-backed agent execution path. It is not the whole APXM workspace;
the `apxm` coordinator owns repo composition, while `server`, `os`, `auth`,
and `studio` own their own planes.

The public surface here is a typed IR plus runtime contracts that let higher
level systems express *what* they want executed while the runtime decides *how*
to admit, schedule, and dispatch it. The vLLM glue remains in this repo as one
backend path, not as the repo identity.

## What is in this repo

- **AIS dialect** (`crates/machine/`) — the Rust-owned operation contract and
  generated metadata consumed by every frontend.
- **Compiler** (`crates/compiler/`) — compiler-owned `FrontendGraph`, the
  canonical Rust AIR validator/printer, MLIR pipeline, and the Python and
  TypeScript authoring packages that lower through it.
- **Runtime** (`crates/runtime/`) — executor, handlers, backend adapters
  (LLM, local, tool); `apxm-backends` holds the vLLM-fork glue.
- **Tools** (`crates/tools/`) — `apxm-cli` and developer clients. The HTTP server
  lives in the `server` repo.
- **`apxm-project/vllm`** — graph-aware vLLM fork. In the APXM coordinator
  workspace it is checked out as `workspace/vllm`; override with
  `APXM_VLLM_DIR` when running APXM standalone.

## Getting started

Use [Dekk](https://github.com/randreshg/dekk) as the authority CLI. Every
build, test, and run goes through it so the env contract, target dir, and
process accounting stay consistent.

```bash
git clone https://github.com/apxm-project/agents
cd agents
dekk agents install --no-interactive
dekk agents doctor                            # verify environment
dekk agents build                             # release build
```

`dekk agents doctor` is the first command of every session. It prints the
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
    immediately without a reboot. `dekk agents doctor` reports whether `bwrap` is
    available. See
    [`docs/integrations/sandbox-acp-seam.md`](docs/integrations/sandbox-acp-seam.md).

## Releases

Release work also goes through Dekk:

```bash
dekk agents release check     # readiness checks; no publishing
dekk agents release dist      # writes archives + SHA256SUMS under .apxm/releases
dekk agents release publish   # dry run by default; pass --yes to publish with gh
```

`release dist` packages eligible private release surfaces and checksums under
`.apxm/releases/vX.Y.Z`. Prototype/internal packages are excluded. GitHub
publishing uses `gh`, never creates a release unless `--yes` is explicit, and
the mutating path requires repository visibility to be exactly `PRIVATE`.
Private Python publication is a separate, fail-closed
`dekk agents release python` action requiring an exact externally supplied
registry manifest, detached signature, allowed-signers file, and signer
identity. No registry endpoint is committed or inferred.

## Cargo feature flags

Most of the crate-level Cargo features that survived the RT-8 flag audit are
storage/tooling swaps, not behavior gates on the observed path:

| Feature | Crate(s) | Default | What it does |
| --- | --- | --- | --- |
| `metrics` | `apxm-backends`, `apxm-runtime` | on | Request recording (`RequestMetrics`/`MetricsTracker`) for every LLM call. Made default-on by RT-8 so the observed path equals the default build — it is no longer possible to build apxm without request recording. |
| `dashmap` / `sqlite` | `apxm-runtime` | on | Swap the concurrent-map / durable-storage backend implementation; both compiled in by default. |
| `no-trace` | `apxm-core`, `apxm-runtime` | off | Strips tracing instrumentation for a smaller/faster build; use only when tracing overhead is unacceptable and you don't need the spans. |
| `embeddings` | `apxm-backends` | off | Pulls in `fastembed` for local embedding generation; off by default (heavy dependency, not every deployment needs it). |
| `test-utils` | `apxm-runtime` | off | Test-only helpers; never enable in a release build. |

Enable a non-default feature with `cargo build --features <name>` (or via the
dependent crate's own feature flags.

## Documentation

- [`docs/README.md`](docs/README.md) — full docs index.
- [`docs/pxm/readme.md`](docs/pxm/readme.md) — PXM theory and APXM's origin
  story.
- [`docs/agent-topology-boundary.md`](docs/agent-topology-boundary.md) —
  hard rule: agent hierarchy and reachability are policy outside the runtime.
- [`docs/compiler/pipeline.md`](docs/compiler/pipeline.md) — compiler pass
  pipeline.
- [`crates/compiler/frontend/README.md`](crates/compiler/frontend/README.md) —
  Rust, Python, and TypeScript frontend contract.
- [`docs/backends/vllm.md`](docs/backends/vllm.md) — APXM/vLLM contract.
- [`docs/backends/storage-layout.md`](docs/backends/storage-layout.md) —
  where APXM puts large files (HF cache, image store, artifacts).
- Run `dekk agents --help` and `dekk agents ops list` for live CLI and AIS
  references.

## Contributing and license

See [`CONTRIBUTING.md`](CONTRIBUTING.md) for the clone flow, the lifecycle
workflow that every non-trivial session routes through, and the commit-message
rules. Released under the [MIT License](LICENSE); the vLLM fork at
`apxm-project/vllm` is Apache-2.0.

For coding agents (Claude Code, Codex CLI, Cursor, Aider, Gemini): read
[`AGENTS.md`](AGENTS.md) or [`CODEX.md`](CODEX.md) (same text; Codex CLI is
wired to `AGENTS.md` in `.agents.json`) or [`CLAUDE.md`](CLAUDE.md) (Claude
Code) before doing any work.
The 6-skill lifecycle — `context` → `plan` →
`execute-plan` → `simplify` → `finish` → `commit` — is
the project-wide pattern.
