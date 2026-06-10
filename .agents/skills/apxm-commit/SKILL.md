---
name: apxm-commit
description: Commit gate — runs apxm-simplify + apxm-finish first, drafts message in repo log style, lints it, and commits only with explicit user approval. Pushes to main only with explicit approval; never --force; never --no-verify. Does not open PRs.
user-invocable: true
---

# APXM Commit

Load `_shared/apxm-agent-operating-rules.md` (commit/push discipline)
and `_shared/apxm-commit-message-rules.md` (message contract) before
any commit or push. Both are non-negotiable.

## What this skill does

1. **Run `apxm-simplify` and `apxm-finish` first.** Refuse to proceed
   if either reported failures.
2. **Verify scope**: one logical slice, not a grab-bag. Stage
   selectively — never `git add -A`.
3. **Verify branch**: `git branch --show-current` must not be `main` or
   a detached HEAD. If on `main`, branch and re-run.
4. **Review the change**: read `git status --short`, `git diff --stat`,
   and `git diff --staged` before drafting.
5. **Confirm no unrelated changes staged**. Unstage with
   `git reset HEAD -- <path>` if needed.
6. **Draft the commit message** per `_shared/apxm-commit-message-rules.md`.
   Allowed types: `feat fix perf refactor docs test chore bench eval prereg`.
   `planNN` scope is valid only for `prereg(...)` / `eval(...)`.
7. **Lint the draft**:
   `echo "<message>" > /tmp/apxm-commit-msg && dekk apxm commit-lint /tmp/apxm-commit-msg`.
   Fix any finding before proceeding.
8. **Commit** with a HEREDOC:
   ```bash
   git commit -m "$(cat <<'EOF'
   feat(<scope>): <subject>

   <body>
   EOF
   )"
   ```
   The `commit-msg` hook (installed by `dekk apxm install-hooks`)
   runs the same lint at commit time. If it blocks, fix the message —
   never `--no-verify`.
9. **If a pre-commit hook fails**: fix the underlying issue, re-stage,
   make a **new** commit. Never `--amend` to bypass; never `--no-verify`.
10. **If pushing**: confirm branch ≠ `main`, then
    `git push -u origin <branch>` (first push) or `git push`.
    Never `--force` without explicit user request.

## Out of scope

- **PR creation.** This skill never runs `gh pr create`. The user opens
  PRs through their own flow when they want one.

## Anti-patterns

- `git add -A` / `git add .`. Always name files.
- `git commit --amend` on a commit that already passed a hook.
- `--no-verify` to bypass a hook.
- Pushing to `main`. Always branch.
- `git push --force` without explicit approval.

## Prerequisite gates

`apxm-finish` must pass before this skill commits. That means
`dekk apxm test`, `dekk apxm doctor`,
`python3 tools/scripts/check_no_legacy_vllm.py --strict`, and
`dekk apxm skills status` (if `.agents/` changed) all clean.
