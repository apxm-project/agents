# Contributing to APXM

APXM is a graph-aware dispatch and scheduling layer for vLLM. The runtime,
the MLIR dialect, the Python frontend, and the vendored vLLM fork at
`external/vllm` live here. This guide covers working on this repo.

## Getting set up

APXM is a Rust workspace with a Python frontend and an MLIR-based compiler.
The supported install path is [Dekk](https://github.com/randreshg/dekk).

```bash
git clone https://github.com/apxm-project/apxm
cd apxm
git submodule update --init --recursive    # apxm-project/vllm under external/vllm
dekk apxm install --no-interactive
dekk apxm doctor
```

`dekk apxm install` creates a repo-local conda environment with MLIR/LLVM 22
and pinned tooling, builds the Rust workspace, and installs the Python
frontend in editable mode. `dekk apxm doctor` is mandatory on every session
start — it prints the resolved environment and refuses to continue if
anything is misaligned (wrong `CARGO_TARGET_DIR`, missing MLIR, stale image
store, missing gateway key, etc.).

## Authority CLI

`dekk apxm` is the only sanctioned entry point. Do not invoke `cargo`,
`docker`, `srun`, `sbatch`, or `python tools/scripts/*` directly — the env
contract and process accounting depend on the wrapper. If a needed action
is not yet wrapped, add a `dekk apxm` subcommand in `.dekk.toml` instead of
shelling out.

The common cadences during development:

```bash
dekk apxm doctor                # session start
dekk apxm build                 # release build of apxm-cli
dekk apxm build-dialect         # rebuild MLIR after .td or C++ shim edits
dekk apxm codegen               # regen Python frontend bindings after .td edits
dekk apxm test                  # workspace tests (excludes compiler + cli)
dekk apxm test-cli              # cli-only (preserves MLIR-linked binary)
dekk apxm test-python-frontend  # pytest the Python frontend
```

Edits to a `.td` file or a TableGen-emitted C++ shim require both
`dekk apxm build-dialect` and `dekk apxm codegen` before the Rust workspace
will compile or the Python frontend will see the new op.

## Lifecycle workflow

Every non-trivial session routes through six lifecycle skills. They are
checkpoints, not new bodies of content — the rules live under
`.agents/skills/_shared/`.

1. `apxm-context` — prime the session (`dekk apxm doctor`,
   `.agents/project.md`, the relevant `_shared/` rule, memory recall).
2. `apxm-plan` — write a plan before implementing. Required for
   changes touching more than three files, modifying a public API or AIS
   op, introducing a claim, or needing GPU allocation.
3. `apxm-execute-plan` — drive the plan to completion with
   focused per-phase verification, no scope creep.
4. `apxm-simplify` — remove copied `_shared/` text, weak
   abstractions, referential comments, and over-large skill bodies before
   declaring done.
5. `apxm-finish` — pre-claim gate: focused `dekk apxm test`,
   `dekk apxm doctor`, release checks, secrets scan, and
   artifact-placement check.
6. `apxm-commit` — commit and push gate: no auto-commit,
   no push without explicit approval, PRs only for pushed work, push to
   `main` only when explicitly authorized.

Skip `apxm-context` and `apxm-plan` only for typos or single-line edits.
Never skip `apxm-finish` or `apxm-commit`.

## Commit messages

Commit subjects follow the rules at
[`.agents/skills/_shared/apxm-commit-message-rules.md`](.agents/skills/_shared/apxm-commit-message-rules.md):
allowed types, no AI attribution, no `planNN` scope outside
`prereg(...)`/`eval(...)`, no `wip` or `fix stuff` subjects. Use
`dekk apxm commit-lint` when you want the repository checker explicitly.

## Submitting a change

1. Create a feature branch off `main` by default. Short, kebab-case
   branch names (e.g. `fix-skill-server-double-escape`,
   `add-call-skill-op`). Work directly on `main` or push to `main` only
   when explicitly authorized.
2. Keep commits small and self-contained. The subject line is at most
   72 characters, imperative mood, and follows the rules above.
3. Run the build and the relevant focused tests before opening a PR.
4. In the PR description, state **what** changed and **why**. Link to
   the related design doc, preregistration, or backlog task. If the
   change touches a compiler pass, the runtime, or a public API, note
   the impact on existing skill artifacts.
5. AI-assisted PRs are welcome. The review and test bar is the same.

## Creating a release

Releases are cut from a clean, synced `main` checkout:

```bash
dekk apxm release check
dekk apxm release dist
dekk apxm release publish --yes
```

`release check` validates git sync, version consistency, changelog coverage,
tag availability, bundled skill manifests, Dekk doctor, generated bindings,
Python tests, release notes, and release builds.
`release dist` writes Python, binary, and source archives plus `SHA256SUMS`
under `.apxm/releases/vX.Y.Z`. `release publish` uses `gh`; without `--yes`
it prints the exact release assets and exits without changing GitHub. PyPI
upload is explicit and separate: `dekk apxm release pypi --yes`.

## Working on the vLLM fork

`external/vllm` is a git submodule pointing at
[`apxm-project/vllm`](https://github.com/apxm-project/vllm) on branch
`apxm-rebase-v0.21.0`. Never edit upstream files inside the submodule
without a cherry-pick plan onto that branch.

The supported serving path is the Dekk-controlled Docker image flow:

```bash
dekk apxm vllm doctor
dekk apxm vllm docker-build --image apxm-vllm-runtime:<TAG> --base-image <VLLM_BASE>
dekk apxm vllm docker-save  --image apxm-vllm-runtime:<TAG>

cp deploy/vllm/zoo.example.toml deploy/vllm/zoo.toml   # then edit
dekk apxm vllm zoo-cache-warm                          # CPU-only shared HF download
dekk apxm vllm zoo-apply                               # submits Slurm jobs
dekk apxm vllm service-list                            # watch readiness
dekk apxm vllm service-exec <NAME> -- dekk apxm execute <GRAPH.py>
```

Configure `.apxm/config.toml` so `data.vllm.hf_cache` and
`data.vllm.model_roots` point at shared storage every Slurm compute node
can read at the same path. `APXM_VLLM_HF_HOME` and
`APXM_VLLM_MODEL_ROOTS` are one-shell overrides, not an invitation to put
large model weights in `$HOME`. The zoo manifest is the operator
surface. See
[`docs/backends/storage-layout.md`](docs/backends/storage-layout.md) for
the placement rules.

## Reporting bugs

Open an issue at <https://github.com/apxm-project/apxm/issues> with:

- The version (`dekk apxm doctor` output is helpful).
- The minimal AIR or Python frontend reproduction.
- The expected behavior, the observed behavior, and any session
  directory paths or stack traces.

For security-sensitive issues, follow [`SECURITY.md`](SECURITY.md) instead
of a public issue.

## Adding a new skill

The SSOT for skill content is `.agents/`. To add a skill:

1. Add `.agents/skills/<name>/SKILL.md` from the template in the
   `apxm-skill-authoring` skill.
2. Run `dekk apxm skills status` to confirm registration.
3. Run `dekk apxm skills generate --target all` to regenerate
   `AGENTS.md`, `CLAUDE.md`, `CODEX.md`, `.agents.json`, `.cursorrules`,
   `.github/copilot-instructions.md`, and other generated agent surfaces
   (see `.agents/skills/README.md`).
4. Commit the skill file and the regenerated outputs in one commit.

Never edit `AGENTS.md`, `CLAUDE.md`, `CODEX.md`, `.agents.json`,
`.cursorrules`, or `.github/copilot-instructions.md` by hand — they are
generated from `.agents/`.

## Code of conduct

This project follows the [Contributor Covenant 2.1](CODE_OF_CONDUCT.md).
By participating you agree to abide by it.

## License

APXM is MIT-licensed (see [`LICENSE`](LICENSE)). By contributing, you
agree that your contribution is licensed under the same MIT terms.
