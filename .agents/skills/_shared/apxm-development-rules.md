# Shared rule — APXM development rules

Load before any session that will edit code, run builds, run tests, or
configure backends. When writing code load `_shared/apxm-comment-rules.md`;
when writing tests load `_shared/apxm-test-rules.md`.

## Authority CLI

- `dekk agents` is the only sanctioned entry point for build, test,
  compile, execute, codegen, doctor, backend, vLLM, MCP, processes.
- Never invoke `cargo`, `docker`, `srun`, `sbatch`, or
  `python tools/scripts/cargo.py` directly. Going through Dekk preserves the env
  contract (`CARGO_TARGET_DIR`, `MLIR_DIR`, `LD_LIBRARY_PATH`, etc.).
- If a needed action isn't wrapped, add a Dekk command in `.dekk.toml`
  rather than shelling out.

## Build environment

- `CARGO_TARGET_DIR=/tmp/apxm-target-$USER`. `/home` is shared WekaFS
  (50+ tenants); building there contends with other users and randomly
  fails (ENOSPC, stale-rustc-cache SIGBUS).
- `MLIR_DIR` / `LLVM_DIR` point at the dekk-managed MLIR/LLVM 22 conda
  env (path `{project}/.dekk/env`).
- `LLM_GATEWAY_KEY` comes from the shell `env:LLM_GATEWAY_KEY`; never
  hard-code, never commit.

## Cadences

- Edited a `.td` file (TableGen op) or TableGen-emitted C++ shim?
  `dekk agents build-dialect` **then** `dekk agents codegen` before
  building the Rust workspace or running Python frontend tests.
- Iterating? Prefer focused commands:
  `dekk agents test -p <crate>` over `dekk agents test-all`.
  `dekk agents check` for a fast type-check, `dekk agents fmt` to format,
  `dekk agents clippy` to lint (deny-warnings).
- Pre-PR? `dekk agents test-all` + `dekk agents test-cli` (CLI requires the
  MLIR-linked binary, hence the separate command).

## Ownership

- **`apxm-core`** owns the AIS dialect. Every other crate consumes it.
  Never define an op outside `apxm-core`.
- Canonical pass list:
  `crates/compiler/pipeline/src/passes/pipeline.rs::build_pass_list()`.
  Add new passes only there.
- Attribute names: canonical enum in `apxm-core`. Python kwargs, MLIR
  attrs, Rust executors all resolve through it. See the
  `feedback_attribute_dual_naming` incident.
- vLLM fork lives at `external/vllm` (submodule, branch
  `apxm-rebase-v0.21.0`, upstream `apxm-project/vllm`). Edits there require
  a cherry-pick plan and the G1 build/smoke gate.

## Reuse-first

- Before inventing a path string, check `apxm.contract.RepoLayout` /
  `apxm.contract.build_layout()`.
- Before adding a new script, look in `tools/scripts/` — many entrypoints
  already exist (`cargo.py`, `vllm.py`, `release.py`,
  `apxm_mcp_install.py`). Keep public names in
  `.dekk.toml`; put larger implementations in a script-local package.
  The shared APXM/vLLM operational names live in the `apxm` Python package at
  `crates/compiler/frontend/python/`.

## Targeted verification

After each phase of work, run the smallest correct check:

- Need a fast compile-only signal? `dekk agents check`.
- Touched one crate's source? `dekk agents test -p <crate>`.
- Touched the CLI? `dekk agents test-cli`.
- Touched a `.td`? `dekk agents build-dialect && dekk agents codegen`,
  *then* the test commands.
- Touched the Python frontend? `dekk agents test-python-frontend`.
- Touched a vLLM zoo manifest? `dekk agents vllm zoo-status`.

Run `dekk agents doctor` if anything in `dekk env`, the conda env, or the
binary toolchain feels off.
