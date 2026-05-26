---
name: apxm-fork-vllm-rebase
description: Use when rebasing the external/vllm fork onto a new upstream tag, cherry-picking APXM commits, or resolving conflicts in the fork. Covers the G1 build/smoke gate.
user-invocable: true
---

# APXM vLLM Fork Rebase

The APXM-vLLM fork lives at `external/vllm` (submodule), on branch
`apxm-rebase-v0.21.0`, upstream `apxm-project/vllm`. It holds upstream
v0.21.0 plus 5 APXM commits.

Load `_shared/apxm-development-rules.md` before broad work.

## Rebase rhythm

1. **Plan first** — invoke `apxm-plan`. A vLLM fork rebase touches a
   public boundary (the HTTP routes APXM consumes); it always needs a
   written plan.
2. **Identify the APXM commits** to carry forward (currently 5; see
   `apxm_fork_rebase_v0_21_0` memory).
3. **Branch off the new upstream tag** in the fork:
   ```bash
   cd external/vllm
   git fetch upstream
   git checkout -b apxm-rebase-v<NEW> v<NEW>
   ```
4. **Cherry-pick the APXM commits** in order. Resolve conflicts
   carefully — upstream may have moved files; do not silently drop
   APXM behavior.
5. **G1 build/smoke gate** (non-negotiable before merging the new
   branch as the submodule pointer):
   ```bash
   dekk apxm vllm doctor
   dekk apxm vllm docker-build --image apxm-vllm:rebase-test \
     --base-image <UPSTREAM_TAG>
   dekk apxm vllm docker-save --image apxm-vllm:rebase-test
   # In a fresh service allocation:
   dekk apxm vllm zoo-apply <test manifest>
   dekk apxm vllm zoo-status
   ```
   Confirm `/v1/apxm/scheduler` reports the expected policy and
   `/v1/apxm/graphs/register` accepts a probe graph.
6. **Update the submodule pointer** in the parent repo only after
   G1 passes. Commit:
   `chore(vllm): rebase external/vllm onto v<NEW>`.

## Rules

- Never edit upstream files inside `external/vllm` outside a
  cherry-pick plan. Use the submodule's branch — never bypass via
  detached HEAD edits.
- Never `git push --force` to `apxm-project/vllm`. Push to a new branch
  and open a PR against the fork's `main` if needed.
- The 4 `/v1/apxm/*` routes are the public contract — see
  `apxm_vllm_boundary` memory. Breaking them breaks every claim-bearing
  bench.
- BaseHTTPMiddleware is forbidden in the fork — see
  `feedback_basehttpmiddleware_breaks_chat`. Use raw ASGI middleware
  on scope/receive/send instead.

## Diagnostics

- `git -C external/vllm log --oneline -10`
- `git -C external/vllm rev-parse HEAD` — must match the parent's
  submodule pointer.
- `dekk apxm vllm doctor` — confirms the fork build is consumable.

## Anti-patterns

- Updating the submodule pointer before G1.
- Force-pushing to the fork.
- Cherry-picking without re-verifying the `/v1/apxm/*` routes.
- Re-introducing BaseHTTPMiddleware (silently breaks chat completions).
