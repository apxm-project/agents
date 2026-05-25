# APXM Agent Skills

Skills are the entry points coding agents use when working in this
repository. Each skill is a short orchestrator (`SKILL.md`, ≤100 lines)
that points at one or more shared rules in `_shared/`.

## Source of truth

- `.agents/project.md` — the SSOT body. Edit there.
- `.agents/skills/_shared/*.md` — shared rules loaded by skills.
- `.agents/skills/<name>/SKILL.md` — individual skills.

The generated files (`AGENTS.md`, `CLAUDE.md`, `.agents.json`,
`.cursorrules`, `.github/copilot-instructions.md`) come from running
`dekk apxm skills generate --target all`. Never edit them by hand.

## Lifecycle skills (the workflow backbone)

Run in order for any non-trivial session:

1. `apxm-context` — prime the session.
2. `apxm-plan` — design before implementing.
3. `apxm-execute-plan` — execute the approved plan.
4. `apxm-simplify` — remove avoidable complexity.
5. `apxm-finish` — pre-claim gate (tests, doctor, lint, secrets).
6. `apxm-commit` — pre-PR gate (no auto-commit, no push to main).

## Domain skills

- `apxm-vllm-service` — APXM-vLLM service operation (existing).
- `apxm-evaluation-artifacts` — artifact placement under `.apxm/`
  (existing).
- `apxm-compile-and-execute` — compile graphs and run `.apxmobj`.
- `apxm-mlir-pass-development` — add/modify MLIR passes.
- `apxm-preregistration` — draft and commit a `docs/preregistrations/`
  entry before claim-bearing runs.
- `apxm-claim-evidence` — write-ups and claim cards after a run.
- `apxm-priority-lane-bench` — priority-lane benchmark workflow.
- `apxm-review-council-bench` — review-council benchmark workflow.
- `apxm-fork-vllm-rebase` — rebase the `external/vllm` fork.
- `apxm-model-zoo-operate` — operate the vLLM zoo manifests.
- `apxm-backend-add` — register new APXM backends.
- `apxm-ais-op-design` — design-before-code for new AIS ops.
- `apxm-skill-authoring` — add/edit agent skills.
- `apxm-mcp-server` — work on the APXM MCP server.

## Shared rules

- `_shared/apxm-agent-operating-rules.md` — commit & push discipline,
  Slurm safety, secrets, no referential comments.
- `_shared/apxm-development-rules.md` — authority CLI, build env,
  ownership, codegen cadence, reuse-first.
- `_shared/apxm-no-legacy-rules.md` — the 12 lint rules (no fallbacks,
  no `service-start`/`-adopt`, no `apxm_endpoints_available`-flag).
- `_shared/apxm-evaluation-rules.md` — artifact placement, claim-
  bearing runs, paired-arm cache-salt scoping.
- `_shared/apxm-preregistration-rules.md` — template, naming, append-
  only norm.
- `_shared/apxm-storage-layout-rules.md` — `/home` is shared WekaFS,
  config-resolver first-wins, HF cache deletion.

## How to add a skill

See `apxm-skill-authoring` for the rules. In short:

1. Add `.agents/skills/<name>/SKILL.md` from the template.
2. `dekk apxm skills status` — confirm registration.
3. `dekk apxm skills generate --target all` — regenerate config files.
4. Commit both the skill and the regenerated outputs.
