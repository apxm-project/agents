---
name: apxm-commit
description: Commit gate — runs apxm-simplify + apxm-finish first, drafts message in repo log style, lints it with dekk apxm commit-lint, commits at a clean stopping point, and pushes only when authorized. Never --force. Does not open PRs.
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
9. **If commit creation fails**: fix the underlying issue, re-stage,
   re-run the relevant Dekk check, then create a new commit.
10. **If pushing**: push to `main` only when explicitly authorized by
    the user; otherwise push the current feature branch with
    `git push -u origin <branch>` (first push) or `git push`.
    Never `--force` without explicit user request.

## Out of scope

- **PR creation.** This skill never runs `gh pr create`. The user opens
  PRs through their own flow when they want one.

## Anti-patterns

- `git add -A` / `git add .`. Always name files.
- `git commit --amend` on a pushed commit.
- Skipping `dekk apxm commit-lint` for a non-trivial message.
- Pushing to `main` without explicit user authorization.
- `git push --force` without explicit approval.

## Prerequisite gates

`apxm-finish` must pass before this skill commits. That means
`dekk apxm test`, `dekk apxm doctor`, relevant release checks, and
`dekk apxm skills status` (if `.agents/` changed) all clean.
