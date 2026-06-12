---
name: apxm-commit
group: Lifecycle
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
   Allowed types: `feat fix perf refactor docs test chore bench eval prereg sec style`.
   `planNN` scope is valid only for `prereg(...)` / `eval(...)`. No
   AI-attribution trailers (`Co-Authored-By: Claude`, `Generated with …`).
7. **Lint the draft (blocking, always)**:
   `echo "<message>" > /tmp/apxm-commit-msg && dekk apxm commit-lint /tmp/apxm-commit-msg`.
   Fix every finding before proceeding — never bypass.
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
10. **Pushing requires explicit approval.** Do not push as part of
    committing. Push only when the user explicitly asks; then push the
    current feature branch (`git push -u origin <branch>` first time,
    else `git push`). Push to `main` only with explicit authorization.
    Never `--force`.

## Out of scope

- **PR creation.** This skill never runs `gh pr create`. The user opens
  PRs through their own flow when they want one.

## Anti-patterns

See `_shared/apxm-agent-operating-rules.md` for the git list (no
`git add -A`, no `--amend` on pushed commits, no push to `main` or
`--force` without approval). Skill-specific: never skip
`dekk apxm commit-lint`; never leave an AI-attribution trailer in the
message.

## Prerequisite gates

`apxm-finish` must pass before this skill commits. That means
`dekk apxm test`, `dekk apxm doctor`, relevant release checks, and
`dekk apxm skills status` (if `.agents/` changed) all clean.
