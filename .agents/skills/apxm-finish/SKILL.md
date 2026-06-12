---
name: apxm-finish
group: Lifecycle
description: Pre-claim gate — runs focused dekk apxm test, doctor, release checks when relevant, secrets scan, and artifact-placement check before any claim of completion. Refuses to claim done until all pass.
user-invocable: true
---

# APXM Finish

Load `_shared/apxm-agent-operating-rules.md` before running this gate.

Enforces that "completion" claims are backed by checks, not by agent
confidence. Run before any verbal "done", PR-ready, or handoff to user.

## What this skill does

Run these in order. If any fail, do **not** claim completion:

1. **Run `apxm-simplify` first** if not already.
2. **Focused tests for touched crates**:
   - `dekk apxm test -p <crate>` per crate that changed.
   - `dekk apxm test-cli` if `crates/tools/apxm-cli/` changed.
   - `dekk apxm test-python-frontend` if
     `crates/compiler/apxm-frontend/python/` changed.
3. **Doctor**: `dekk apxm doctor`. Catches a drifted conda env or
   stale MLIR.
4. **Release checks** when release-facing files changed:
   `dekk apxm release check` with the smallest justified skip set for
   unrelated expensive checks.
5. **Commit-message lint** for any queued commits:
   `dekk apxm commit-lint --range origin/main..HEAD`.
6. **Skills status** if anything under `.agents/` changed:
   `dekk apxm skills status`. Confirm generated agent files and
   `.agents.json` are coherent.
7. **`git status --short`** and **`git diff --stat`**. Read every
   line. Nothing should be unexpected.
8. **Secrets scan** if settings/env/deploy files changed:
   ```bash
   git diff --staged | grep -iE 'LLM_GATEWAY_KEY|oauth_token|hf_token|HUGGING_FACE_HUB_TOKEN|sk-[a-zA-Z0-9]{20,}'
   ```
   Should return nothing. Also confirm `.claude/settings.local.json`
   is **not** staged (it's gitignored for a reason).
9. **Artifact placement** if any new path is added:
    - All generated artifacts under `.apxm/`?
    - Nothing under `examples/**/results/`, `examples/**/runs/`, or
      `examples/**/sessions/`?
    - `RepoLayout` used for any new path?
10. **Report concretely** to the user:
    - What changed (per file/crate).
    - What passed (each command + exit code).
    - What's local-only (build artifacts, local config).
    - What remains (any follow-up tasks, deferred items).

## Output template

```
## Finish report

### Changed
- crates/runtime/...: <one-line summary>
- ...

### Passed
- dekk apxm test -p <crate>: green
- dekk apxm doctor: green
- dekk apxm release check: green
- skills status: coherent

### Local-only
- /tmp/apxm-target-$USER (build cache, not committed)

### Remaining
- <follow-up task 1>
- <follow-up task 2>
```

## Anti-patterns

- "Tests pass" without naming which tests.
- Skipping the secrets scan because "I'm sure it's clean".
- Marking done when a release or commit check warned but you didn't
  address it.
- "I didn't run X because it's slow" — slow is not a reason to skip;
  scope down the command instead.

## Next step

Hand off to `apxm-commit` when the user is ready to stage/commit/push.
Never auto-commit from `apxm-finish`.
