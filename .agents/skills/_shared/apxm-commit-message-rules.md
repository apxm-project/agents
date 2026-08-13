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
`sec`, `style`.

`sec` is for security fixes/hardening; `style` is for formatting-only
changes (e.g. `cargo fmt`). Exactly these. Deprecated spellings the lint
will reject with a suggestion:

- `Add …`, `Update …`, `Harden …` (no type prefix) → `<type>(<scope>): …`

## Scope rules

- Shape: `[A-Za-z][A-Za-z0-9_/-]*`. Common examples: `runtime`,
  `compiler`, `core`, `cli`, `frontend`, `backends`,
  `benchmarks`, `tools`, `mlir`, `dispatch`, `lint`, `agents`, `tests`,
  `metrics`, `examples`.
- Use subsystem scopes such as `compiler`, `runtime`, `frontend`, or
  `examples`; do not scope commits by a plan number.
- Tool/agent names (`ultrathink`, `claude`, `codex`, `cursor`, `aider`,
  `copilot`) are not valid scopes — name the subsystem instead.

Wrong: `feat(run42): cohort stamping` — scope by *subsystem*:
`feat(benchmarks): cohort stamping for cross-system run`.

Wrong: `fix(ultrathink): foo` — name the actual subsystem touched.

Right: `feat(benchmarks): add telecom workload`.

## Excluded from subject

- Filler: `wip`, `fix stuff`, `misc`, `tweaks`, `…`, `tmp`.
- Tool / agent names: `ultrathink`, `claude`, `codex`, `cursor` as a
  scope. (`fix(ultrathink): …` is wrong — name the subsystem instead.)
- Emoji (`✨`, `🚀`, `🤖`, etc.).

## Excluded from body

- AI-attribution lines:
  - `Co-Authored-By: Claude …`
  - `Generated with [Claude Code]`
  - `🤖 Generated with …`
- Referential phrasing tied to ephemeral context:
  - "as discussed", "per the chat", "per the user", "Claude said".
- Cross-references to plans, tickets, or skills. Those belong in the
  change description or issue tracker, not in the commit body.

## Required in body (when a body is present)

- Explain **why**, not what. The diff already shows what.
- One paragraph is usually enough. Wrap at 72 chars.
- If the change is trivial (typo, rename, one-liner), no body needed.

## Per-type expectations

- `bench(<scenario>)` / `feat(benchmarks)`: scope by the workload name,
  not by a plan or ticket id.
- `chore(agents)`: agent-facing contract and generated-instruction
  maintenance.
- `docs(<scope>)`: when touching `.agents/` SSOT, remember to
  `dekk agents skills generate --target all` so generated agent files stay synced.

## Enforcement

`dekk agents commit-lint` is the gate (`<message-file>`, `--current` for
HEAD, or `--range A..B` for a series). The `commit` skill runs it on
the drafted message as a **blocking** step before every commit — this is
what catches the banned AI-attribution trailers and untyped subjects. If
the lint blocks a message, fix the message — never `--no-verify`, never
bypass.
