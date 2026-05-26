# APXM — agent-facing project memory

This file is the single source of truth (SSOT) for every coding agent that
enters this repository (Claude Code, Codex CLI, Cursor, Aider, Gemini, etc.).
Both `CLAUDE.md` and `AGENTS.md` in the repo root are generated from this
file by `dekk apxm skills generate`. Edit this file, then regenerate.

## 1. What APXM is

APXM is a graph-aware **dispatch + scheduling layer** for vLLM, with an
AMD-aligned **CPU/GPU split** so that planning, validation, and analysis stay
on CPU while inference runs on GPU. Its public surface is an MLIR dialect
(AIS) plus a Rust runtime plus a vLLM fork that accepts dispatch hints.

Do **not** describe APXM as "an agent framework", "an LLM orchestrator", or
"a multi-agent runtime" — that mischaracterizes the project and confuses
new contributors. The correct anchor is: *graph-aware dispatch for vLLM*.

## 2. Authority CLI

`dekk apxm` is the only sanctioned entry point. Never invoke `cargo`,
`docker`, `srun`, `sbatch`, `python tools/scripts/cargo.py`, `service-start`,
`service-adopt`, or `run-vllm-slurm.sh` directly. Always go through Dekk so
the env contract, target dir, and process accounting stay consistent.

Command groups (see `dekk apxm --help` for the live list):

- **Build & Test**: `build`, `build-dialect`, `test`, `test-cli`,
  `test-python-frontend`, `codegen`, `clean`, `scrub-rustc-cache`
- **Compilation**: `compile`, `execute`, `run`, `decompile`
- **Authoring**: `validate`, `analyze`, `explain`, `gui`, `tokenize`
- **Configuration**: `doctor`, `backend`, `vllm`, `agent`, `tool`, `cache`,
  `process`, `mcp`, `server`, `commit-lint`, `install-hooks`
- **Discovery**: `ops`, `template`
- **vLLM operate**: `dekk apxm vllm {doctor, probe, cache-warm,
  docker-build, docker-save, docker-load, zoo-apply, zoo-status, zoo-scale,
  zoo-cache-warm, service-list, service-status, service-exec, service-stop,
  check-no-legacy, verify-cross-shard-pins}`

If a needed action isn't yet wrapped, **add a Dekk command** in `.dekk.toml`
rather than shelling out — that is the project-wide pattern.

## 3. Lifecycle workflow

Every non-trivial session ceremonially routes through 6 lifecycle skills.
They are thin orchestrators (≤100 lines each) — they do not contain rule
content themselves; they point at `_shared/` rules.

1. **`/apxm-org:apxm-context`** — prime the session: `dekk apxm doctor`,
   read `.agents/project.md`, pull the relevant `_shared/` rule, recall
   memory, confirm subsystem ownership. Run before any work touching >1
   file.
2. **`/apxm-org:apxm-plan`** — write a plan before implementing. Required
   for changes that touch >3 files, modify a public API/AIS op, introduce
   a claim, or need GPU allocation.
3. **`/apxm-org:apxm-execute-plan`** — drive an approved plan to
   completion with `TaskCreate`/`TaskUpdate`, focused per-phase
   verification, no scope creep.
4. **`/apxm-org:apxm-simplify`** — remove copied `_shared/` text, weak
   abstractions, referential comments, and over-large skill bodies before
   declaring done.
5. **`/apxm-org:apxm-finish`** — pre-claim gate: run focused
   `dekk apxm test`, `dekk apxm doctor`, `check_no_legacy_vllm.py
   --strict`, secrets scan, artifact-placement check. Refuse to claim
   "done" until all pass.
6. **`/apxm-org:apxm-commit`** — pre-commit/pre-push gate: enforce the
   user's commit rules — no auto-commit, no push without explicit
   approval, PRs only for pushed work, never push to `main`, never
   `--no-verify`.

This is the *ironbear pattern* — each skill is a checkpoint, not a body of
new content. Skills inside the lifecycle can invoke domain skills (e.g.
`apxm-vllm-service` for vLLM service operations).

## 4. Repo layout

- **`crates/`** — Rust workspace.
  - `crates/core/` — APXM core: AIS dialect definitions (the public IR
    contract), graph types, attribute taxonomy.
  - `crates/compiler/` — passes pipeline, frontend Python bindings,
    TableGen-driven MLIR.
  - `crates/runtime/` — executor, handlers, backend adapters (LLM, local,
    tool).
  - `crates/runtime/apxm-backends/` — LLM provider implementations,
    vLLM-fork glue.
  - `crates/tools/apxm-cli/` — `apxm` binary subcommands.
- **`external/vllm/`** — git submodule, vLLM fork on branch
  `apxm-rebase-v0.21.0` (upstream v0.21.0 + 5 APXM commits at
  `apxm-project/vllm`). Never edit upstream files there directly without a
  cherry-pick plan.
- **`tools/scripts/`** — Python wrappers Dekk calls into (`vllm.py`,
  `cargo.py`, `check_no_legacy_vllm.py`, `apxm_mcp_install.py`).
- **`crates/compiler/apxm-frontend/python/apxm/`** — installable `apxm`
  Python package. `apxm.contract` owns the APXM/vLLM operational names
  (env vars, routes, dataclasses, `build_layout()`); `apxm.data_config`
  resolves the `.apxm/` data buckets.
- **`deploy/vllm/`** — `zoo.toml` manifests (operator state) and
  `run-vllm.sh` (the unified deploy script — never `run-vllm-slurm.sh`).
- **`docs/`** — design docs for the core runtime.
- **`.agents/`** — this SSOT plus `_shared/` rules, lifecycle skills,
  domain skills, and `domains/` navigation README-only directories.
- **`.apxm/`** — generated artifacts (gitignored): benchmark results,
  evaluation runs, service registry, compiler diagnostics, vLLM images.

### Companion repos

The APXM org is split by ownership boundary. This repo stays focused on
graph-aware dispatch for vLLM; companion repos carry the children that were
born from APXM for their own scopes:

- `apxm-project/apxm-eval` — born from APXM to keep preregistrations,
  benchmarks, claim cards, and paper drafts independent from the runtime.
- `apxm-project/apxm-libs` — born from APXM to package compiled skills and
  manifests that `apxm-server` can load via `APXM_SKILLS_PATH`.
- `apxm-project/apxm-os` — born from APXM to supervise long-lived agents and
  their manifests outside the core compiler/runtime repo.
- `apxm-project/apxm-gui` — born from APXM to ship the axum + React dashboard
  as a standalone binary; `dekk apxm gui` shells to it when it is on `PATH`.
- `apxm-project/vllm` — born from APXM to carry the graph-aware vLLM fork that
  accepts APXM dispatch hints; this repo vendors it as `external/vllm`.

Skills under `.agents/skills/` are agent-tooling for working *on* APXM and
stay here unless a later PR explicitly migrates them to a child repo.

## 5. Build, test, codegen

Required env (set by `dekk apxm doctor` + the conda env):

- `CARGO_TARGET_DIR=/tmp/apxm-target-$USER` — `/home` is shared WekaFS,
  builds there contend with 50+ other users and randomly fail.
- `MLIR_DIR`, `LLVM_DIR` — set by the dekk env (MLIR/LLVM 22).
- `LLM_GATEWAY_KEY` — comes from `env:LLM_GATEWAY_KEY`; never commit.

Standard cadences:

```bash
dekk apxm doctor                # always run on session start (or via hook)
dekk apxm build                 # release build of apxm-cli (driver+metrics)
dekk apxm build-dialect         # rebuild MLIR after .td or C++ shim edits
dekk apxm codegen               # regen Python frontend bindings after .td edits
dekk apxm test                  # workspace tests (excluding compiler+cli)
dekk apxm test-cli              # cli-only (preserves MLIR-linked binary)
dekk apxm test-python-frontend  # pytest the Python frontend
```

Run **focused** checks during iteration. Full `test-all` is for pre-PR
verification only.

When you edit a `.td` file or a TableGen-emitted C++ shim, you must run
`dekk apxm build-dialect` first **then** `dekk apxm codegen` before the
Rust workspace will compile or the Python frontend will see the new op.

## 6. Artifact placement

All generated artifacts live under `.apxm/` (repo-local, gitignored).
Never put benchmark CSVs, session directories, `.apxmobj` files, compiler
diagnostics, evidence manifests, vLLM logs, or per-run configs under
`examples/`, `docs/`, or repo root.

Use the helper:

```python
from apxm.contract import RepoLayout, build_layout
layout = build_layout(__file__)  # paths for benchmarks/evaluation/vllm-*
```

Canonical sub-locations:

- Benchmark results: `.apxm/benchmarks/results/`
- Evaluation runs: `.apxm/evaluation/<scenario>/runs/<UTC>/`
- vLLM image store: `.apxm/vllm-images/`
- vLLM service registry: `.apxm/vllm-services/`
- HF cache: `data.vllm.hf_cache` in `.apxm/config.toml`, or
  `APXM_VLLM_HF_HOME` (must be visible from every Slurm compute node).

If you find a generated artifact under `examples/` or `docs/`, move it to
the matching `.apxm` location and patch whatever script wrote it there —
do **not** add an ignore guard to mask the bug.

## 7. No-legacy / no-fallback contract

Hard-fail at config time, never `or env or default` chains. The lint at
`tools/scripts/check_no_legacy_vllm.py` enforces 12 rules — all are
project policy:

1. **`legacy-service-start`** — `dekk apxm vllm service-start` is removed.
   Use `zoo apply` against a `deploy/vllm/zoo*.toml` manifest.
2. **`legacy-service-adopt`** — `service-adopt` removed. Write a `zoo.toml`
   entry, `zoo apply`.
3. **`legacy-run-vllm-slurm`** — `run-vllm-slurm.sh` deleted. Use
   `deploy/vllm/run-vllm.sh`.
4. **`hardcoded-port-8916`** — never literal `8916` outside the allocator
   range default; use `_allocate_port()` or a manifest-supplied port.
5. **`apxm-endpoints-available-flag`** — no
   `apxm_endpoints_available`-style flags that paper over a missing fork.
6. **`resolver-last-resort`** — no resolver "last resort" branches.
7. **`resolver-rr-fallback`** — no silent round-robin fallback in the
   resolver.
8. **`scheduling-policy-fcfs`** — no FCFS scheduling literal in code that
   should consume the configured policy.
9. **`or-env-or-default-chain`** — no `cfg or env or "default"` chains
   that hide missing required config.
10. **`already-exists-skipping`** — no silent "skipping, already exists"
    branches; hard-fail or surface explicitly.
11. **`shell-model-specific-default`** — no model-specific defaults baked
    into shell wrappers.
12. **`hardcoded-dispatch-field-literal`** — promote dispatch field names
    to `graph_attrs::*` constants; never literal strings in handlers.

Run before any PR: `python3 tools/scripts/check_no_legacy_vllm.py --strict`
(or `dekk apxm vllm check-no-legacy`).

Promote contract strings (env var names, route paths, response markers)
to constants. The `metrics_keys::*` and `graph_attrs::*` modules are the
source of truth — refer to them, don't duplicate the literal.

## 8. Preregistration before claims

Quality/perf claim workflow (preregistrations, benchmarks, claim cards,
write-ups) lives in `apxm-project/apxm-eval`. Claim-bearing runs against
the core runtime should reference that repo for the preregistration
template, evidence layout, and `apxm-finish`-style claim gating. The
`apxm-finish` lifecycle skill in this repo no longer enforces a
preregistration check — that gate lives next to the eval harness.

## 9. AIS dialect ownership

**`apxm-core` is the only crate that defines AIS ops.** Compiler passes,
runtime handlers, the Python frontend, and the vLLM fork are all
consumers. After editing any `.td` file (TableGen op definition) or a
TableGen-emitted C++ shim:

```bash
dekk apxm build-dialect   # rebuild MLIR (TableGen + C++ + Rust)
dekk apxm codegen         # regenerate Python frontend bindings
```

Both are non-negotiable: skipping either produces silent type drift
between the Rust runtime, the Python frontend, and the MLIR layer.

The canonical pass list lives at
`crates/compiler/apxm-compiler/src/passes/pipeline.rs::build_pass_list()`.
Add new passes there (and only there) so the pipeline stays in one place.

Attribute names must be a single source of truth — see the
`feedback_attribute_dual_naming` incident: the canonical enum lives in
`apxm-core`; Python kwargs, MLIR attrs, and Rust executors must all
resolve through it, never via duplicated string literals.

## 10. Storage layout

`/home` is shared WekaFS (9.1 TiB, 50+ tenants). It is **not** personal
disk:

- **Build outputs**: `/tmp/apxm-target-$USER` (456 GiB local). `/home`
  contention has caused random ENOSPC and stale-rustc-cache SIGBUS in
  the past.
- **HF cache + vLLM images + service registry**: under
  `~/.cache/huggingface-apxm-vllm/hub/` and `.apxm/vllm-images/`; these
  must be visible from every Slurm compute node at the same path.
  `dekk apxm vllm doctor` prints the resolved layout.
- **HF cache deletion**: blobs are root-owned (docker runs as root). A
  plain `rm` silently succeeds without freeing space — use `sudo rm`.
- **Config resolver**: `load_scoped` is **first-wins, not merge**. A
  project-local `.apxm/config.toml` with only a data-dir override will
  shadow the backend block in `~/.apxm/config.toml` and produce a silent
  "no backends configured". Either fully merge or use one config file.

See `docs/backends/storage-layout.md` for the full contract and the
supported migration procedure.

## 11. Boundaries (read before any potentially destructive action)

### Never, under any circumstance

- **Auto-commit**. Ask for explicit user approval before every `git
  commit`, even if the user said "commit this" in a previous turn.
- **Push without explicit approval**. The user must explicitly ask for
  the push. Approval is per-action, not per-session.
- **Push to `main`**. Always work on a feature branch; PRs are how
  pushed work reaches `main`.
- **`git push --force`** anywhere.
- **`--no-verify`** to bypass hooks. If a hook fails, fix the root
  cause; never re-stage and bypass. The `commit-msg` hook installed
  by `dekk apxm install-hooks` enforces
  `_shared/apxm-commit-message-rules.md` (allowed types, no AI
  attribution, no `planNN` scope outside `prereg(...)`/`eval(...)`,
  no `wip`/`fix stuff` subjects).
- **`scancel`** a Slurm job owned by `raherrer`. Always allocate a
  fresh service job alongside.
- **Commit secrets**: `LLM_GATEWAY_KEY`, OAuth tokens, HF tokens.
  `apxm-finish` scans for these.
- **Commit generated artifacts**: `.apxm/`, `zoo.toml`, `slurm-*.out`,
  `.apxmobj` files, benchmark CSVs.
- **Bypass `dekk apxm`** for normal work — raw `cargo`/`docker`/`srun`
  break the env contract.
- **Delete or `git checkout --` unexplained files/branches**. They may
  be the user's in-progress work. Investigate first.
- **`git rebase --no-edit`**, `--no-gpg-sign`, `git commit --amend` on
  pushed commits.

### Confirm before doing

- Any `sudo` invocation.
- Any `docker run/build/rm/rmi` or image-tag mutation.
- Any Slurm submission (`sbatch`, `srun`, `salloc`).
- Any `gh pr create/comment/merge`, posting to Slack/email/webhooks.
- Any edit to `~/.apxm/config.toml`, `~/.bashrc`, `~/.gitconfig`,
  systemd units, cron entries, or `.claude/settings.local.json`.
- Cross-crate refactors and changes to public APIs / AIS ops — these
  warrant `apxm-plan` first.

### When in doubt

Read the relevant doc, recall memory, and ask. The cost of one
clarifying question is far smaller than the cost of an unintended
push, an overwritten branch, or a tainted benchmark.

<!-- BEGIN SKILLS INVENTORY -->
## Available Skills

Before editing CARTS sources, scan the Skills inventory below and read the SKILL.md for any whose description matches your task.

### Other

| Skill | Description | Path |
| --- | --- | --- |
| `apxm-ais-op-design` | Use before adding or modifying an AIS op in apxm-core. Enforces the design-before-code gate, the canonical-attribute rule, and the build-dialect + codegen cadence. | `.agents/skills/apxm-ais-op-design/SKILL.md` |
| `apxm-backend-add` | Use when registering a new APXM inference backend (cloud, on-prem, or local). Enforces hard-fail-at-config-time and the no-legacy / no-fallback contract on resolver behavior. | `.agents/skills/apxm-backend-add/SKILL.md` |
| `apxm-commit` | Commit gate — runs apxm-simplify + apxm-finish first, drafts message in repo log style, lints it, and commits. Auto-commit allowed; never pushes to main; never --force; never --no-verify. Does not open PRs. | `.agents/skills/apxm-commit/SKILL.md` |
| `apxm-compile-and-execute` | Use when compiling APXM graphs, running .apxmobj artifacts, or executing AIR/IR through the runtime. Enforces dekk apxm as the authority CLI and correct artifact placement under .apxm/. | `.agents/skills/apxm-compile-and-execute/SKILL.md` |
| `apxm-context` | Prime an APXM session before broad work — runs doctor, reads project.md and the relevant _shared rules, surfaces subsystem ownership, and recalls APXM memory. Run at the start of any session that will touch >1 file or any non-trivial change. | `.agents/skills/apxm-context/SKILL.md` |
| `apxm-execute-plan` | Drive an APXM plan to completion without scope creep. Tracks phases with the harness's task tracker, runs focused per-phase verification, refuses to add features beyond the plan, and surfaces blockers immediately. Invoke only after apxm-plan produces an approved plan. | `.agents/skills/apxm-execute-plan/SKILL.md` |
| `apxm-finish` | Pre-claim gate — runs focused dekk apxm test, doctor, no-legacy lint, secrets scan, and artifact-placement check before any claim of completion. Refuses to claim done until all pass. | `.agents/skills/apxm-finish/SKILL.md` |
| `apxm-fork-vllm-rebase` | Use when rebasing the external/vllm fork onto a new upstream tag, cherry-picking APXM commits, or resolving conflicts in the fork. Covers the G1 build/smoke gate. | `.agents/skills/apxm-fork-vllm-rebase/SKILL.md` |
| `apxm-mcp-server` | Use when working on or registering the APXM MCP server (mcp/apxm_server.py). Each tool shells dekk apxm <cmd>; registration goes through dekk apxm mcp install. | `.agents/skills/apxm-mcp-server/SKILL.md` |
| `apxm-mlir-pass-development` | Use when adding, modifying, or reordering MLIR passes in the APXM compiler pipeline. Enforces the single pass-list source of truth, the AIS-core ownership rule, and the build-dialect + codegen cadence after .td edits. | `.agents/skills/apxm-mlir-pass-development/SKILL.md` |
| `apxm-model-zoo-operate` | Use when adding, scaling, or probing models in the vLLM zoo (deploy/vllm/zoo*.toml). Enforces the docker-load → cache-warm → service-start → service-exec order and zoo-apply over service-start. | `.agents/skills/apxm-model-zoo-operate/SKILL.md` |
| `apxm-plan` | Produce a written plan before non-trivial APXM implementation. Required for changes touching >3 files, modifying a public API or AIS op, or needing Slurm GPU allocation. Enforces APXM-specific gates (AIS-op-vs-compose decision, dialect-codegen impact). | `.agents/skills/apxm-plan/SKILL.md` |
| `apxm-simplify` | Pre-finish review pass — remove copied _shared text, weak abstractions, referential comments, and over-large skill bodies before claiming completion. Mandatory before apxm-finish and any commit. | `.agents/skills/apxm-simplify/SKILL.md` |
| `apxm-vllm-service` | Use when building, launching, probing, or running APXM workloads against the Dockerized APXM-vLLM backend, especially on Slurm compute nodes. Enforces Dekk as the authority CLI, persistent service allocations, image-store reuse, and service-exec for commands that need the vLLM endpoint. | `.agents/skills/apxm-vllm-service/SKILL.md` |

<!-- END SKILLS INVENTORY -->
