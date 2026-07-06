# APXM Agent Skills

Skills are the entry points coding agents use when working in this
repository. Each skill is a short orchestrator (`SKILL.md`, ≤100 lines)
that points at one or more shared rules in `_shared/`.

## Source of truth

- `.agents/project.md` — the SSOT body. Edit there.
- `.agents/skills/_shared/*.md` — shared rules loaded by skills.
- `.agents/skills/<name>/SKILL.md` — individual skills.

The generated files (`AGENTS.md`, `CLAUDE.md`, `CODEX.md`, `.agents.json`,
`.cursorrules`, `.github/copilot-instructions.md`) come from running
`dekk agents skills generate --target all`. Never edit them by hand. Today
`AGENTS.md` and `CODEX.md` share the same body; Codex CLI is configured on
`AGENTS.md` in `.agents.json` (see `.agents/domains/meta/README.md`).

## Lifecycle skills (the workflow backbone)

Run in order for any non-trivial session:

1. `context` — prime the session.
2. `plan` — design before implementing.
3. `execute-plan` — execute the approved plan.
4. `simplify` — remove avoidable complexity.
5. `finish` — pre-claim gate (tests, doctor, release checks, secrets).
6. `commit` — commit/push gate; push to `main` only with
   explicit user authorization.

## Domain skills

- `vllm-service` — APXM-vLLM service operation (existing).
- `compile-and-execute` — compile graphs and run `.apxmobj`.
- `frontend-implementation` — compiler frontend work across Rust
  `AirModule`, TypeScript `@apxm/frontend`, Python `apxm`, and AIR lowering.
- `mlir-pass-development` — add/modify MLIR passes.
- `fork-vllm-rebase` — rebase the `external/vllm` fork.
- `model-zoo-operate` — operate the vLLM zoo manifests.
- `backend-add` — register new APXM backends.
- `ais-op-design` — design-before-code for new AIS ops.
- `mcp-server` — work on the APXM MCP server.
- `design-docs` — gate overclaim/citation-drift in conceptual `docs/`.

Benchmark, preregistration, claim-evidence, and evaluation-artifact
skills live in the companion repo `apxm-project/eval`.

## Shared rules

`.agents/skills/_shared/` here is the **canonical source** for shared
rules. The cross-repo subset is mirrored byte-for-byte into
`eval/.agents/skills/_shared/`; the rest are repo-local.

Synced across apxm and eval (edit here, copy to eval):

- `_shared/apxm-agent-operating-rules.md` — commit & push discipline
  (no auto-commit, no push without approval), Slurm safety, secrets.
- `_shared/apxm-commit-message-rules.md` — type/scope/subject format,
  banned trailers, `dekk agents commit-lint` enforcement.
- `_shared/apxm-comment-rules.md` — per-language comment conventions.
- `_shared/apxm-test-rules.md` — test authoring (placement, what to pin).
- `_shared/apxm-evaluation-rules.md` — claim-bearing-run discipline.
- `_shared/apxm-preregistration-rules.md` — preregistration discipline.
- `_shared/apxm-storage-layout-rules.md` — `/home` is shared WekaFS,
  config-resolver first-wins, HF cache deletion.

Repo-local (not synced):

- `_shared/apxm-development-rules.md` — authority CLI, build env,
  ownership, codegen cadence, reuse-first. (apxm only)
- `_shared/apxm-self-host-rules.md` — contract for self-hosted dev
  workflows (APXM builds itself). (apxm only)
- `eval/.agents/skills/_shared/apxm-current-layout-rules.md` — eval-only
  current-layout lint discipline. (eval only)

## How to add a skill

Authoring rules (this repo's own SSOT convention):

- **Frontmatter**: `name`, `description` (one-line, agent-discoverable),
  optional `user-invocable: true`.
- **Body ≤100 lines**, a thin orchestrator. Open with
  `Load _shared/<rule>.md before broad work.` — never inline `_shared` text.
  If it grows past 100 lines, move detail to `docs/` or
  `.agents/domains/<area>/README.md`.
- List authority commands (not raw `cargo`/`docker`), anti-patterns (the
  project's hard-won lessons), and a `See also`. No referential content
  ("for planNN", ticket numbers). Must work for both Claude and Codex.
- Don't duplicate an existing skill — extend it instead.

Steps:

1. Add `.agents/skills/<name>/SKILL.md` from the convention above.
2. `dekk agents skills status` — confirm registration.
3. `dekk agents skills generate --target all` — regenerate config files.
4. Commit both the skill and the regenerated outputs.
