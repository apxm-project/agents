---
name: worktree
description: Create and manage isolated git worktrees for parallel work
user-invocable: true
---

# Worktree

Use `dekk worktree` to manage git worktrees with automatic environment setup.

## Commands

- `dekk worktree create <branch>` — create a new worktree with a fresh branch
- `dekk worktree create <branch> --base main` — branch from main
- `dekk worktree create <branch> --existing` — checkout an existing branch
- `dekk worktree create <branch> --no-setup` — skip automatic environment setup
- `dekk worktree list` — show all worktrees and their dekk status
- `dekk worktree remove <name>` — remove a worktree
- `dekk worktree remove <name> --force` — force-remove even with modifications
- `dekk worktree prune` — clean up stale references

## Project Integration

Worktrees are fully integrated with dekk's project command system:

1. Create a worktree: `dekk worktree create feature-x --base main`
2. Change to it: `cd ../$(basename $PWD)-worktrees/feature-x`
3. All project commands work immediately — `dekk` walks up to find `.dekk.toml`
4. The `.agents/` source of truth is shared across all worktrees (same repo)

## Key Behaviors

- Worktrees share the same `.dekk.toml` and `.agents/` (same git repo)
- Each worktree gets its own independent working directory for builds, tests, etc.
- `dekk worktree create` auto-runs `dekk setup` to prepare the environment
- By default worktrees are created at `../<repo>-worktrees/<branch>`
- Branch names with slashes (e.g., `feature/login`) become `feature-login` in the path

## When to Use

- Working on a feature while keeping main branch clean for reference
- Running tests on one branch while developing on another
- Reviewing a PR without stashing your current work
- Parallel builds or CI-like isolation
