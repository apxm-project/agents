# Shared rule — APXM agent operating rules

Load this file at the start of any APXM session and re-load before any
git mutation, push, PR, or destructive action. These rules override
project conventions and skill defaults.

## Commit & push discipline

- **Auto-commit is allowed.** When work is at a clean stopping point
  (apxm-finish gate passed, message drafted in repo style), commit
  without an extra approval round-trip.
- **Push directly to a feature branch is allowed by default** once the
  local commit is sound. No PR step is required unless the user asks.
- **Push to `main` only when explicitly authorized by the user.**
- **No `git push --force`** anywhere. Even on a feature branch, ask
  first.
- **Do not skip commit/release checks.** Run the relevant `dekk apxm`
  gate and fix the root cause when it fails.
- **No `git commit --amend`** on pushed commits. New commit instead.
- **No `git add -A` / `git add .`** — name files explicitly so secrets
  and generated artifacts don't slip in.
- **No interactive rebase** (`-i`); skill flow doesn't support it.
- **No `gh pr create`** unless the user explicitly asks for a PR.

## Slurm & shared infra

- **Never `scancel`** a Slurm job owned by `apxm`. Always allocate a
  fresh service job alongside an existing dev-node allocation.
- Confirm before `sbatch`, `srun`, `salloc`.
- Confirm before any `sudo`.
- Confirm before any `docker run/build/rm/rmi` outside the Dekk wrapper.
- HF cache blobs are root-owned; deletion needs `sudo rm` — plain `rm`
  silently no-ops.

## Secrets

- Never commit `LLM_GATEWAY_KEY`, OAuth tokens, HF tokens, or
  `.claude/settings.local.json`.
- `apxm-finish` scans for these before any commit.

## Code style (operating discipline)

- Default to no comments; only justify *why*, never *what*.
- **No referential comments**: never reference plans, tickets, prior
  conversations, or "fix for X" in code. The commit message and PR
  description own that context.
- Promote contract strings (env-var names, route paths, response
  markers) to constants. The `metrics_keys::*` and `graph_attrs::*`
  modules are the source of truth.
- No backwards-compat shims unless explicitly requested. Delete
  unused code outright.

## Investigation before deletion

If you find unexpected state (an unfamiliar file, branch, lock file, or
config), **investigate first**. It is almost always the user's
in-progress work. Never delete or `git checkout --` it to "clean up".

## When in doubt

Ask. The cost of one clarifying question is far smaller than the cost
of an unintended push, an overwritten branch, or a tainted benchmark
that has to be re-run on GPU time you don't own.
