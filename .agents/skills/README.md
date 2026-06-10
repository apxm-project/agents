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
`dekk apxm skills generate --target all`. Never edit them by hand.

## Lifecycle skills (the workflow backbone)

Run in order for any non-trivial session:

1. `apxm-context` — prime the session.
2. `apxm-plan` — design before implementing.
3. `apxm-execute-plan` — execute the approved plan.
4. `apxm-simplify` — remove avoidable complexity.
5. `apxm-finish` — pre-claim gate (tests, doctor, lint, secrets).
6. `apxm-commit` — pre-commit/pre-push gate; push to `main` only with
   explicit user authorization.

## Domain skills

- `apxm-vllm-service` — APXM-vLLM service operation (existing).
- `apxm-compile-and-execute` — compile graphs and run `.apxmobj`.
- `apxm-goal-orchestrator` — create, start, and follow bounded APXM goal
  orchestration passes through `dekk apxm goal` or workflow MCP tools.
- `apxm-mlir-pass-development` — add/modify MLIR passes.
- `apxm-fork-vllm-rebase` — rebase the `external/vllm` fork.
- `apxm-model-zoo-operate` — operate the vLLM zoo manifests.
- `apxm-backend-add` — register new APXM backends.
- `apxm-ais-op-design` — design-before-code for new AIS ops.
- `apxm-mcp-server` — work on the APXM MCP server.
- `apxm-design-docs` — gate overclaim/citation-drift in `docs/design/`.

Benchmark, preregistration, claim-evidence, and evaluation-artifact
skills live in the companion repo `apxm-project/apxm-eval`.

## Shared rules

- `_shared/apxm-agent-operating-rules.md` — commit & push discipline,
  Slurm safety, secrets, no referential comments.
- `_shared/apxm-development-rules.md` — authority CLI, build env,
  ownership, codegen cadence, reuse-first.
- `_shared/apxm-no-legacy-rules.md` — the 12 lint rules (no fallbacks,
  no `service-start`/`-adopt`, no `apxm_endpoints_available`-flag).
- `_shared/apxm-storage-layout-rules.md` — `/home` is shared WekaFS,
  config-resolver first-wins, HF cache deletion.

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
2. `dekk apxm skills status` — confirm registration.
3. `dekk apxm skills generate --target all` — regenerate config files.
4. Commit both the skill and the regenerated outputs.
