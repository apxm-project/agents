# agents — agent-facing project memory

The `.agents/` tree is the single source of truth (SSOT) for every coding
agent that enters this repository (Claude Code, Codex CLI, Cursor, Aider,
Gemini, etc.).

Repo-root instruction files are generated from `.agents/project.md` (plus
skill registration under `.agents/skills/`) by `dekk agents skills generate`.
Edit `.agents/` sources, then run `dekk agents skills generate --target all`.
Do not edit generated roots (`AGENTS.md`, `CLAUDE.md`, `CODEX.md`,
`.cursorrules`, `.github/copilot-instructions.md`, `.agents.json`) by hand.

**Which file for which tool:** `CLAUDE.md` is the Claude Code entrypoint.
`AGENTS.md` is the canonical portable instructions file (Codex CLI uses it
per `.agents.json`; ACP session output for the `codex` profile also writes
`AGENTS.md` into per-node dirs). `CODEX.md` duplicates the `AGENTS.md` body for
workflows that look for a Codex-named file. Cursor reads `.cursorrules`;
GitHub Copilot reads `.github/copilot-instructions.md`.

## 1. What agents is

`agents` is the APXM abstract-machine repo: AIS dialect, compiler, runtime,
capability contracts, context handling, permissions, orchestration, CLI, and
the profile-backed agent execution path. It is not the whole APXM workspace;
the `apxm` coordinator owns repo composition, while `server`, `os`, `auth`,
and `studio` own their own planes.

Do **not** describe `agents` as the APXM coordinator, the HTTP server, the OS
host plane, or only "vLLM dispatch". The correct anchor is: *the abstract
machine and runtime contracts for APXM agents*.

## 2. Authority CLI

`dekk agents` is the only sanctioned entry point. Never invoke `cargo`,
`docker`, `srun`, `sbatch`, or `python tools/scripts/cargo.py` directly.
Always go through Dekk so the env contract, target dir, and process
accounting stay consistent.

Command groups (see `dekk agents --help` for the live list):

- **Build & Test**: `build`, `build-dialect`, `test`, `test-cli`,
  `test-python-frontend`, `codegen`, `clean`, `scrub-rustc-cache`
- **Compilation**: `compile`, `execute`, `run`, `decompile`
- **Authoring**: `validate`, `analyze`, `explain`, `gui`, `tokenize`
- **Workflows**: `workflow`; MCP callers use `workflow_start`,
  `workflow_status`, `workflow_events`, `workflow_cancel`, and
  `prompt_as_workflow`
- **Configuration**: `doctor`, `backend`, `vllm`, `agent`, `tool`, `cache`,
  `process`, `mcp`, `server`, `commit-lint`
- **Discovery**: `ops`, `template`
- **Release**: `release {check, dist, publish, pypi}`
- **vLLM operate**: `dekk agents vllm {doctor, probe, cache-warm,
  docker-build, docker-save, docker-load, zoo-apply, zoo-status, zoo-scale,
  zoo-cache-warm, service-list, service-status, service-exec, service-stop}`

If a needed action isn't yet wrapped, **add a Dekk command** in `.dekk.toml`
rather than shelling out — that is the project-wide pattern.

Checked-in `.apxmw` workflows use `dekk agents workflow` or `workflow_start`;
natural-language workflow drafts use `prompt_as_workflow` and remain proposals
until APXM validates and admits them.

## 3. Lifecycle workflow

Every non-trivial session ceremonially routes through 6 lifecycle skills.
They are thin orchestrators (≤100 lines each) — they do not contain rule
content themselves; they point at `_shared/` rules.

1. **`context`** — prime the session: `dekk agents doctor`,
   read `.agents/project.md`, pull the relevant `_shared/` rule, recall
   memory, confirm subsystem ownership. Run before any work touching >1
   file.
2. **`plan`** — write a plan before implementing. Required
   for changes that touch >3 files, modify a public API/AIS op, introduce
   a claim, or need GPU allocation.
3. **`execute-plan`** — drive an approved plan to
   completion with the current harness task tracker, focused per-phase
   verification, no scope creep.
4. **`simplify`** — remove copied `_shared/` text, weak
   abstractions, referential comments, and over-large skill bodies before
   declaring done.
5. **`finish`** — pre-claim gate: run focused
   `dekk agents test`, `dekk agents doctor`, release checks, secrets scan,
   artifact-placement check. Refuse to claim "done" until all pass.
6. **`commit`** — commit/push gate: enforce the
   user's commit rules — no auto-commit, no push without explicit
   approval, PRs only for pushed work, push to `main` only when explicitly
   authorized.

This is the *ironbear pattern* — each skill is a checkpoint, not a body of
new content. Skills inside the lifecycle can invoke domain skills (e.g.
`vllm-service` for vLLM service operations).

## 4. Repo layout

- **`crates/`** — Rust workspace.
  - `crates/core/` — APXM core: AIS dialect definitions (the public IR
    contract), graph types, attribute taxonomy.
  - `crates/compiler/` — passes pipeline, frontend Python bindings,
    TableGen-driven MLIR.
  - `crates/runtime/` — executor, handlers, backend adapters (LLM, local,
    tool).
  - `crates/runtime/backends/` — LLM provider implementations,
    vLLM-fork glue.
  - `crates/tools/cli/` — `apxm` binary subcommands.
- **`external/vllm/`** — git submodule, vLLM fork on branch
  `apxm-rebase-v0.21.0` (upstream v0.21.0 + 5 APXM commits at
  `apxm-project/vllm`). Never edit upstream files there directly without a
  cherry-pick plan.
- **`tools/scripts/`** — Python entrypoints Dekk calls into (`cargo.py`,
  `vllm.py`, `release.py`, `apxm_mcp_install.py`). Larger command implementations live in a
  script-local package such as `apxm_release/`.
- **`crates/compiler/frontend/python/apxm/`** — installable `apxm`
  Python package. `apxm.contract` owns the APXM/vLLM operational names
  (env vars, routes, dataclasses, `build_layout()`); `apxm.data_config`
  resolves the `.apxm/` data buckets.
- **`deploy/vllm/`** — `zoo.toml` manifests (operator state) and
  `run-vllm.sh` (the deploy script used by zoo services).
- **`docs/`** — design docs for the core runtime.
- **`.agents/`** — this SSOT plus `_shared/` rules, lifecycle skills,
  domain skills, and `domains/` navigation README-only directories.
- **`.apxm/`** — generated artifacts (gitignored): benchmark results,
  evaluation runs, service registry, compiler diagnostics, vLLM images.

### External dependency: vLLM fork

`apxm-project/vllm` is the graph-aware vLLM fork that exposes five
`/v1/apxm/*` routes and the `vllm_xargs.apxm` request-hint envelope. It
is vendored at `external/vllm` on branch `apxm-rebase-v0.21.0` and is the
only external repo this codebase depends on directly.

Skills under `.agents/skills/` are agent-tooling for working *on* APXM and
stay here.

## 5. Build, test, codegen

Required env (set by `dekk agents doctor` + the conda env):

- `CARGO_TARGET_DIR=/tmp/apxm-target-$USER` — `/home` is shared WekaFS,
  builds there contend with 50+ other users and randomly fail.
- `MLIR_DIR`, `LLVM_DIR` — set by the dekk env (MLIR/LLVM 22).
- `LLM_GATEWAY_KEY` — comes from `env:LLM_GATEWAY_KEY`; never commit.

Standard cadences:

```bash
dekk agents doctor                # always run on session start
dekk agents build                 # release build of apxm-cli (driver+metrics)
dekk agents build-dialect         # rebuild MLIR after .td or C++ shim edits
dekk agents codegen               # regen Python frontend bindings after .td edits
dekk agents test                  # workspace tests (excluding compiler+cli)
dekk agents test-cli              # cli-only (preserves MLIR-linked binary)
dekk agents test-python-frontend  # pytest the Python frontend
```

Run **focused** checks during iteration. Full `test-all` is for pre-PR
verification only.

When you edit a `.td` file or a TableGen-emitted C++ shim, you must run
`dekk agents build-dialect` first **then** `dekk agents codegen` before the
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

## 7. vLLM operating contract

Hard-fail at config time, never `or env or default` chains. Use the zoo
manifest as the operator surface and keep service state explicit.

Run `dekk agents release check` before release work; use focused tests for
ordinary development changes.

Promote contract strings (env var names, route paths, response markers)
to constants. The `metrics_keys::*` and `graph_attrs::*` modules are the
source of truth — refer to them, don't duplicate the literal.

## 8. Owner-local evaluation

Agents-owned offline and observed prompt evaluation lives under
`evaluation/` and is driven through the `dekk agents
offline-prompt-evaluation` and `dekk agents observed-prompt-evaluation`
surfaces. Each bundle carries a `preregistration.json` recording disjoint
case ids, digests, and provenance before execution.

## 9. AIS dialect ownership

**`apxm-core` is the only crate that defines AIS ops.** Compiler passes,
runtime handlers, the Python frontend, and the vLLM fork are all
consumers. After editing any `.td` file (TableGen op definition) or a
TableGen-emitted C++ shim:

```bash
dekk agents build-dialect   # rebuild MLIR (TableGen + C++ + Rust)
dekk agents codegen         # regenerate Python frontend bindings
```

Both are non-negotiable: skipping either produces silent type drift
between the Rust runtime, the Python frontend, and the MLIR layer.

The canonical pass list lives at
`crates/compiler/pipeline/src/passes/pipeline.rs::build_pass_list()`.
Add new passes there (and only there) so the pipeline stays in one place.

Attribute names must be a single source of truth — see the
`feedback_attribute_dual_naming` incident: the canonical enum lives in
`apxm-core`; Python kwargs, MLIR attrs, and Rust executors must all
resolve through it, never via duplicated string literals.

## 10. Capability abstract machine vocabulary

APXM models **capabilities** as the first-class abstract-machine unit.
Each capability composes a **capability_binding** (callable implementation) plus a
**permission policy** (authority). Runtime authority flows through typed
**capability grants** (`grant_*` ids), not bare handler strings.

Canonical terms, reserved aliases, route naming, and schema versions live in
the coordinator glossary:

- `../../docs/context/capability-vocabulary.md` — SSOT for APXM-owned capability
  vocabulary (`CapabilityDefinition`, `CapabilityBinding`, `PermissionPolicy`,
  `CapabilityGrant`, `PermissionOperation`, `PromptPolicy`, …).

When touching capability registry, admission, pack schemas, AIS
`REGISTER_CAPABILITY` / `INV_CAP.capability`, or server `/v1/capability-templates`
routes, read that doc first and keep code, schemas, and UI copy aligned.

## 11. Storage layout

`/home` is shared WekaFS (9.1 TiB, 50+ tenants). It is **not** personal
disk:

- **Build outputs**: `/tmp/apxm-target-$USER` (456 GiB local). `/home`
  contention has caused random ENOSPC and invalid-rustc-cache SIGBUS in
  the past.
- **HF cache + vLLM images + service registry**: under
  `~/.cache/huggingface-apxm-vllm/hub/` and `.apxm/vllm-images/`; these
  must be visible from every Slurm compute node at the same path.
  `dekk agents vllm doctor` prints the resolved layout.
- **HF cache deletion**: blobs are root-owned (docker runs as root). A
  plain `rm` silently succeeds without freeing space — use `sudo rm`.
- **Config resolver**: `load_scoped` is **first-wins, not merge**. A
  project-local `.apxm/config.toml` with only a data-dir override will
  shadow the backend block in `~/.apxm/config.toml` and produce a silent
  "no backends configured". Either fully merge or use one config file.

See `docs/backends/storage-layout.md` for the full contract and the
supported migration procedure.

## 12. Boundaries (read before any potentially destructive action)

### Never, under any circumstance

- **Push to `main`**. Always work on a feature branch. The user moves
  the work to `main` through their own flow.
- **`git push --force`** anywhere without explicit approval.
- **`gh pr create`** unless the user explicitly asks for a PR. The
  commit + push gate stops at push.
- **Skipping commit checks**. Use `dekk agents commit-lint` when a commit
  message needs explicit validation.
- **`scancel`** a Slurm job owned by `apxm`. Always allocate a
  fresh service job alongside.
- **Commit secrets**: `LLM_GATEWAY_KEY`, OAuth tokens, HF tokens.
  `finish` scans for these.
- **Commit generated artifacts**: `.apxm/`, `zoo.toml`, `slurm-*.out`,
  `.apxmobj` files, benchmark CSVs.
- **Bypass `dekk agents`** for normal work — raw `cargo`/`docker`/`srun`
  break the env contract.
- **Delete or `git checkout --` unexplained files/branches**. They may
  be the user's in-progress work. Investigate first.
- **`git rebase --no-edit`**, `--no-gpg-sign`, `git commit --amend` on
  pushed commits.

### Confirm before doing

- Any `sudo` invocation.
- Any `docker run/build/rm/rmi` or image-tag mutation.
- Any Slurm submission (`sbatch`, `srun`, `salloc`).
- Posting to external notification or webhook endpoints.
- Any edit to `~/.apxm/config.toml`, `~/.bashrc`, `~/.gitconfig`,
  systemd units, cron entries, or `.claude/settings.local.json`.
- Cross-crate refactors and changes to public APIs / AIS ops — these
  warrant `plan` first.

### When in doubt

Read the relevant doc, recall memory, and ask. The cost of one
clarifying question is far smaller than the cost of an unintended
push, an overwritten branch, or a tainted benchmark.

<!-- BEGIN SKILLS INVENTORY -->
## Available Skills

### Other

| Skill | Description | Path |
| --- | --- | --- |
| `ais-op-design` | Use before adding or modifying an AIS op in apxm-core. Enforces the design-before-code gate, the canonical-attribute rule, the definitions.rs source-of-truth layer map, and the build-dialect + codegen cadence. | `.agents/skills/ais-op-design/SKILL.md` |
| `backend-add` | Use when registering a new APXM inference backend (cloud, on-prem, or local). Enforces hard-fail-at-config-time resolver behavior. | `.agents/skills/backend-add/SKILL.md` |
| `commit` | Commit gate — runs simplify + finish first, drafts message in repo log style, lints it with dekk agents commit-lint, commits at a clean stopping point, and pushes only when authorized. Never --force. Does not open PRs. | `.agents/skills/commit/SKILL.md` |
| `compile-and-execute` | Use when compiling APXM AIR workflows, running .apxmobj artifacts, or executing AIR/IR through the runtime. Enforces dekk agents as the authority CLI and correct artifact placement under .apxm/. | `.agents/skills/compile-and-execute/SKILL.md` |
| `context` | Prime an APXM session before broad work — runs doctor, reads project.md and the relevant _shared rules, surfaces subsystem ownership, and recalls APXM memory. Run at the start of any session that will touch >1 file or any non-trivial change. | `.agents/skills/context/SKILL.md` |
| `design-docs` | Use when editing conceptual docs under docs/. Gates two shipped failure modes — overclaim (present-tense prose about unwired behaviour) and citation drift (claims with no anchor to shipped code). | `.agents/skills/design-docs/SKILL.md` |
| `execute-plan` | Drive an APXM plan to completion without scope creep. Tracks phases with the harness's task tracker, runs focused per-phase verification, refuses to add features beyond the plan, and surfaces blockers immediately. Invoke only after plan produces an approved plan. | `.agents/skills/execute-plan/SKILL.md` |
| `finish` | Pre-claim gate — runs focused dekk agents test, doctor, release checks when relevant, secrets scan, and artifact-placement check before any claim of completion. Refuses to claim done until all pass. | `.agents/skills/finish/SKILL.md` |
| `fork-vllm-rebase` | Use when rebasing the external/vllm fork onto a new upstream tag, cherry-picking APXM commits, or resolving conflicts in the fork. Covers the G1 build/smoke gate. | `.agents/skills/fork-vllm-rebase/SKILL.md` |
| `frontend-implementation` | Use when changing APXM compiler frontends: Rust FrontendGraph lowering, TypeScript @apxm/frontend, Python apxm_program, frontend codegen, or Studio/source lowering into AIR. | `.agents/skills/frontend-implementation/SKILL.md` |
| `mcp-server` | Use when working on APXM MCP surfaces: the Rust HTTP `/v1/mcp` endpoint, the Rust stdio `mcp-server` binary, or cross-agent MCP registration. Prefer server-owned HTTP MCP for workflow/orchestration control. | `.agents/skills/mcp-server/SKILL.md` |
| `mlir-pass-development` | Use when adding, modifying, or reordering MLIR passes in the APXM compiler pipeline. Enforces the single pass-list source of truth, the AIS-core ownership rule, and the build-dialect + codegen cadence after .td edits. | `.agents/skills/mlir-pass-development/SKILL.md` |
| `model-zoo-operate` | Use when adding, scaling, or probing models in the vLLM zoo (deploy/vllm/zoo*.toml). Enforces docker-load then cache-warm then zoo-apply then service-exec/status; use the zoo surface only. | `.agents/skills/model-zoo-operate/SKILL.md` |
| `plan` | Produce a written plan before non-trivial APXM implementation. Required for changes touching >3 files, modifying a public API or AIS op, or needing Slurm GPU allocation. Enforces APXM-specific gates (AIS-op-vs-compose decision, dialect-codegen impact). | `.agents/skills/plan/SKILL.md` |
| `simplify` | Pre-finish review pass — remove copied _shared text, weak abstractions, referential comments, and over-large skill bodies before claiming completion. Mandatory before finish and any commit. | `.agents/skills/simplify/SKILL.md` |
| `vllm-service` | Use when building, launching, probing, or running APXM workloads against the Dockerized APXM-vLLM backend, especially on Slurm compute nodes. Enforces Dekk as the authority CLI, persistent service allocations, image-store reuse, and service-exec for commands that need the vLLM endpoint. | `.agents/skills/vllm-service/SKILL.md` |

<!-- END SKILLS INVENTORY -->
