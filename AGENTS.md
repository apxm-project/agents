# AGENTS.md

Instructions for AI coding agents (Codex CLI, Aider, Cursor, Claude Code,
and friends) working in this repository. Humans should read
[`CONTRIBUTING.md`](CONTRIBUTING.md) instead.

This file is the canonical, agent-agnostic source for *which operations are
safe to run without asking the user* and *which always need confirmation*.
Agent-specific config (e.g. `.claude/settings.local.json`) mirrors this list;
keep them in sync when this file changes.

## Project shape

- Rust workspace under `crates/` (core, runtime, backends, tools).
- Python frontend under `crates/compiler/apxm-frontend/python`.
- MLIR dialect compiled via `dekk apxm regen`.
- vLLM fork as a git submodule at `external/vllm` (branch
  `apxm-rebase-v0.21.0`); pointer is committed in the parent repo.
- Build artifacts MUST go to `/tmp` because `/home` is shared WekaFS:
  `export CARGO_TARGET_DIR=/tmp/apxm-target-$USER`.

## Safe by default (no confirmation needed)

Agents may run these without asking, in either the repo root or the
submodule.

### Read-only inspection

- File reads (`cat`, `head`, `tail`, dedicated `Read` tools).
- `find`, `grep`, `rg`, `ls`, `stat`, `file`, `wc`.
- All read-only git: `git status`, `git log`, `git diff`, `git show`,
  `git blame`, `git ls-tree`, `git rev-parse`, `git fetch` (no `--prune`),
  `git branch` (no `-D`/`-d`), `git stash list`, `git remote -v`,
  `git submodule status`.
- Submodule equivalents via `git -C external/vllm <read-only command>`.

### Build / test

- `cargo check -p <crate>` and `cargo check --workspace`.
- `cargo clippy -p <crate>` and `cargo clippy --workspace`.
- `cargo test -p <crate>` and `cargo test --workspace` (incl. `--lib`,
  `--doc`, filter strings).
- `cargo build -p <crate>` (release builds are fine; they don't touch
  shared state).
- `pytest` under `crates/compiler/apxm-frontend/python` and
  `examples/python/benchmarks`.
- `python3 -c '...'` for one-shot inspection.
- `dekk apxm doctor`, `dekk apxm ops list` (read-only diagnostics).

### Local file edits

- Editing source files under `crates/`, `examples/`, `tools/`, `docs/`.
- Creating new files in `docs/preregistrations/`, `docs/evaluation/`,
  `docs/paper/`, `docs/claims/` (these are evidence artifacts — see
  "Evidence artifacts" below).

### Local git mutations (still recorded; reviewable)

- `git add <path>` (NOT `git add -A` / `git add .` — too easy to stage
  secrets or unintended files).
- `git commit -m "..."` (commit messages are reviewed in the log; pre-commit
  hooks run).
- `git reset HEAD -- <path>` (unstage; reversible).
- `git stash push` (recoverable).

## Always ask first

These are destructive, shared, or hard to reverse. Confirm with the user
even if a prior task seemed to authorize a similar action.

### Destructive git

- `git push` of any kind, especially `--force` / `+ref`.
- `git rebase` (interactive or otherwise).
- `git reset --hard`, `git checkout -- .`, `git restore --staged .` over
  multiple files, `git clean -f`.
- `git branch -D`, `git tag -d`.
- `git commit --amend` (use a new commit unless the user explicitly says
  amend).
- Anything that bypasses pre-commit hooks: `--no-verify`, `--no-gpg-sign`.

### Shared infrastructure

- `sudo <anything>`.
- Docker: `docker run`, `docker build`, `docker rm`, `docker rmi`,
  anything that touches images or containers.
- Slurm: `sbatch`, `srun`, `salloc`, and **never** `scancel` a job
  belonging to `apxm` (this is a recurring footgun — always allocate a
  fresh service job instead).
- `rm -rf` outside `/tmp`. Inside `/tmp` is fine if the path is
  unambiguously yours.
- GPU operations (`rocm-smi`, `nvidia-smi --reset`, anything that resets
  device state).
- Modifying HuggingFace cache under
  `~/.cache/huggingface-apxm-vllm/hub/` (blobs are root-owned; needs
  `sudo rm`).

### External-facing actions

- `gh pr create`, `gh issue create`, `gh pr comment`, `gh pr merge`,
  posting to Slack/email/webhooks.
- Pushing or publishing anywhere (registries, package indexes, gists,
  pastebins).
- Uploading files to third-party renderers/diagram tools — they may cache
  or index the upload.

### Configuration with global blast radius

- Editing `~/.apxm/config.toml`, `~/.bashrc`, `~/.zshrc`, `~/.gitconfig`,
  systemd units, cron entries.
- Modifying `.claude/settings.local.json` (or equivalent agent config)
  without surfacing the diff first.

## Project-specific rules

These come from prior incidents; please follow them without exception.

### vLLM integration

- `model.id` in APXM config MUST match the vLLM `served_model_name`
  exactly. The bare name (`gpt-oss-120b`), not the HF repo
  (`openai/gpt-oss-120b`). Mismatches produce silent 404s.
- Hard-fail at config time. Never write `or env or default` chains, silent
  skips, `apxm_endpoints_available`-style flags, or resolver "last resort"
  branches. See `tools/scripts/check_no_legacy_vllm.py` for the lint.
- Cache-salt scoping for paired-arm benchmarks must be per `(arm, opt)`,
  never per `(iter, row)` — else the second arm hits the first arm's
  cache and contaminates the comparison.

### Code style

- Default to no comments. Only add one when the *why* is non-obvious.
- Never reference plans/tickets in code comments
  (`// for plan04`, `// from issue #123`).
- Promote contract strings (env var names, route paths, response markers)
  to constants. The `metrics_keys::*` and `graph_attrs::*` modules are
  the source of truth.

### Evidence artifacts

- `docs/preregistrations/*.md` and `docs/claims/*.md` are frozen records of
  experiments. Once committed, don't rewrite history — append a new prereg
  if the protocol changes.
- Power/perf numbers belong in `docs/evaluation/` with a backing prereg.

## Reduce noise in agent prompts

If you're an agent maintainer wiring this repo into a new tool, the
allowlist in `.claude/settings.local.json` is a good starting set. Mirror
the "safe by default" section above into your tool's permission system.
