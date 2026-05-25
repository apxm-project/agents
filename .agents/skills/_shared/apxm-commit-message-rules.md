# Shared rule — APXM commit-message contract

Load this file before drafting any commit message. The lint at
`tools/scripts/check_commit_message.py` is the enforcement layer; this
file is the spec.

## Subject line

```
<type>(<scope>): <subject>
```

- Length: ≤72 chars including type and scope.
- Imperative mood (`add`, `fix`, `extract`), not past tense or gerund.
- No trailing period, no emoji, no shouting (no all-caps subjects).
- One logical change per commit. If the subject needs " and ", split.

## Allowed types

`feat`, `fix`, `perf`, `refactor`, `docs`, `test`, `chore`, `bench`,
`eval`, `prereg`.

Exactly these. Deprecated spellings the lint will reject with a
suggestion:

- `pre-reg`, `preregister`, `preregistration` → `prereg`
- `Add …`, `Update …`, `Harden …` (no type prefix) → `<type>(<scope>): …`

## Scope rules

- Shape: `[A-Za-z][A-Za-z0-9_/-]*`. Common examples: `runtime`,
  `compiler`, `core`, `cli`, `frontend`, `vllm`, `backends`,
  `benchmarks`, `tools`, `mlir`, `dispatch`, `lint`, `agents`, `tests`,
  `metrics`, `examples`, `deploy`, `external/vllm`.
- `planNN` (e.g. `plan04`, `plan09`) is **only** valid for `prereg(...)`
  and `eval(...)`. Never for `feat`, `fix`, `refactor`, `perf`, `chore`.
- Use `plan04` (no hyphen), not `plan-04`.
- Tool/agent names (`ultrathink`, `claude`, `codex`, `cursor`, `aider`,
  `copilot`) are not valid scopes — name the subsystem instead.

Wrong: `feat(plan04): cohort stamping` — scope by *subsystem*:
`feat(benchmarks): cohort stamping for cross-system run`.

Wrong: `fix(ultrathink): foo` — name the actual subsystem touched.

Right: `prereg(plan09): J/req cell — telecom N=20`.

## Forbidden in subject

- Filler: `wip`, `fix stuff`, `misc`, `tweaks`, `…`, `tmp`.
- Tool / agent names: `ultrathink`, `claude`, `codex`, `cursor` as a
  scope. (`fix(ultrathink): …` is wrong — name the subsystem instead.)
- Emoji (`✨`, `🚀`, `🤖`, etc.).

## Forbidden in body

- AI-attribution lines:
  - `Co-Authored-By: Claude …`
  - `Generated with [Claude Code]`
  - `🤖 Generated with …`
- Referential phrasing tied to ephemeral context:
  - "as discussed", "per the chat", "per the user", "Claude said".
- Cross-references to plans / tickets / skills for non-`prereg`/`eval`
  commits. Those belong in the PR description. Exception: `prereg(...)`
  and `eval(...)` bodies may cite the experiment id and the
  preregistration file path.

## Required in body (when a body is present)

- Explain **why**, not what. The diff already shows what.
- One paragraph is usually enough. Wrap at 72 chars.
- If the change is trivial (typo, rename, one-liner), no body needed.

## Per-type expectations

- `prereg(<planNN>)`: subject names the cell/scenario; body cites the
  preregistration filename under `docs/preregistrations/` and the
  metric/N/concurrency design points. No write-up of results — that
  belongs in an `eval(...)` commit later.
- `eval(<scenario>)`: subject names the scenario; body cites the
  preregistration commit SHA and the artifact path under
  `.apxm/evaluation/`.
- `bench(<scenario>)` / `feat(benchmarks)`: scope by the workload name
  (`apxm_review_council`, `apxm_priority_lane`), not by plan id.
- `chore(external/vllm)`: submodule bump; body names the upstream SHA
  and what APXM commits ride on top.
- `docs(<scope>)`: when touching `.agents/` SSOT, remember to
  `dekk apxm skills generate` so `CLAUDE.md` + `AGENTS.md` stay synced.

## Bypass policy

The `commit-msg` hook is installed by `dekk apxm install-hooks`. The
user's operating rules forbid `--no-verify`. If the lint blocks a
message, fix the message — don't bypass.
