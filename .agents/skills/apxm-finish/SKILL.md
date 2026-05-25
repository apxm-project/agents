---
name: apxm-finish
description: Pre-claim gate — runs focused dekk apxm test, doctor, no-legacy lint, secrets scan, artifact-placement check, and preregistration check before any claim of completion. Refuses to claim done until all pass.
user-invocable: true
---

# APXM Finish

Load `_shared/apxm-agent-operating-rules.md`, `_shared/apxm-no-legacy-rules.md`,
and `_shared/apxm-evaluation-rules.md` before running this gate.

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
4. **No-legacy lint**: `python3 tools/scripts/check_no_legacy_vllm.py
   --strict` (or `dekk apxm vllm check-no-legacy`).
5. **Skills status** if anything under `.agents/` changed:
   `dekk apxm skills status`. Confirm CLAUDE.md, AGENTS.md, and
   `.agents.json` are coherent.
6. **`git status --short`** and **`git diff --stat`**. Read every
   line. Nothing should be unexpected.
7. **Secrets scan** if settings/env/deploy files changed:
   ```bash
   git diff --staged | grep -iE 'LLM_GATEWAY_KEY|oauth_token|hf_token|HUGGING_FACE_HUB_TOKEN|sk-[a-zA-Z0-9]{20,}'
   ```
   Should return nothing. Also confirm `.claude/settings.local.json`
   is **not** staged (it's gitignored for a reason).
8. **Artifact placement** if benchmark/eval files were touched:
   - All generated artifacts under `.apxm/`?
   - Nothing under `examples/python/benchmarks/results/`,
     `examples/python/demos/*/runs/`, or `examples/**/sessions/`?
   - `RepoLayout` used for any new path?
9. **Preregistration interlock** if the change is claim-bearing:
   - Matching file exists in `docs/preregistrations/`?
   - Preregistration commit timestamp is *before* the first artifact
     in `.apxm/evaluation/<scenario>/runs/<UTC>/`?
   - Write-up cites the preregistration commit SHA?
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
- check_no_legacy_vllm.py --strict: 0 violations
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
- Claim-bearing work without a preregistration commit.
- Marking done when a hook warned but you didn't address it.
- "I didn't run X because it's slow" — slow is not a reason to skip;
  scope down the command instead.

## Next step

Hand off to `apxm-commit` when the user is ready to stage/commit/push.
Never auto-commit from `apxm-finish`.
