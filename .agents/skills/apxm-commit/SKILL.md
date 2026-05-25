---
name: apxm-commit
description: Pre-PR gate — enforces no auto-commit, no push without explicit approval, no push to main, no --no-verify, and PR-for-pushed-work-only. Drafts commit message in repo log style; asks for explicit user approval before each commit.
user-invocable: true
---

# APXM Commit

Load `_shared/apxm-agent-operating-rules.md` before any commit or push.

The user's commit discipline is **non-negotiable**:

- **No auto-commits.** Ask explicit user approval before every commit.
- **No push without explicit approval.** Per-action, not per-session.
- **No push to `main`.** Always a feature branch + PR.
- **No `--no-verify`.** Fix the hook; never bypass.

## What this skill does

1. **Run `apxm-simplify` and `apxm-finish` first.** Refuse to proceed
   if either reported failures.
2. **Verify scope**: one logical slice, not a grab-bag. Flag cross-crate
   changes for user confirmation. Stage selectively — never `git add -A`.
3. **Verify branch**: `git branch --show-current` must not be `main` or
   a detached HEAD. If on `main`, instruct user to branch and re-run.
4. **Review the change**: read `git status --short`, `git diff --stat`,
   and `git diff --staged` before drafting.
5. **Confirm no unrelated changes staged**. Unstage with
   `git reset HEAD -- <path>` if needed.
6. **Draft the commit message** in repo log style:
   - First line ≤72 chars, imperative.
   - Type prefix: `feat(<scope>)`, `fix(<scope>)`, `refactor(<scope>)`,
     `docs(<scope>)`, `prereg(<plan>)`, `chore(<scope>)`,
     `eval(<scenario>)`, `bench(<scenario>)`.
   - Body explains *why*; the diff shows *what*.
   - No plan/ticket/skill references in the body (those go in PR desc).
7. **Ask the user for explicit approval** of the message before
   running `git commit`. Show the exact message.
8. **Commit** only after approval. Use a HEREDOC:
   ```bash
   git commit -m "$(cat <<'EOF'
   feat(<scope>): <subject>

   <body>
   EOF
   )"
   ```
9. **If a pre-commit hook fails**: fix the underlying issue, re-stage,
   make a **new** commit. Never `--amend` to bypass; never `--no-verify`.
10. **If asked to push**: confirm branch ≠ `main`, confirm with user once
    more, then `git push -u origin <branch>` (first push) or `git push`.
    Never `--force`.
11. **For pushed work**: draft PR title (≤70 chars) and body (Summary +
    Test plan); run `gh pr create` only after explicit approval.

## Anti-patterns

- "I'll just commit this and we can fix later." Ask first.
- `git add -A` / `git add .`. Always name files.
- `git commit --amend` on a commit that already passed a hook.
- `--no-verify` to bypass a hook.
- Pushing to `main`. Always branch + PR.
- PR title/body without explicit approval.

## Output template

```
## Commit ready

Branch: <branch>
Files: <list>

Proposed message:

  <type>(<scope>): <subject>

  <body>

Confirm to commit? (yes / edit / cancel)
```

## Prerequisite gates

`apxm-finish` must pass before this skill commits. That means
`dekk apxm test`, `dekk apxm doctor`,
`python3 tools/scripts/check_no_legacy_vllm.py --strict`, and
`dekk apxm skills status` (if `.agents/` changed) all clean.
