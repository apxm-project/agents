# Agent builder

Gao-local skill for creating APXM agents and composing sub-agents.

Use when the operator asks to create an agent, scaffold an agent folder, add a
specialist/sub-agent, or choose between a single agent, in-agent sub-agent,
or workflow delegation.

Canonical reference: `apxm.agent.v1`. The folder contract below is a direct
read of that schema. `agent.toml` is the only authored manifest; generated
`integrity.toml` carries the tamper-evident seal.

## Agent folder (`apxm.agent.v1`)

```text
<agent-id>/
  agent.toml                    # required — id, version, license, source, compile,
                                 #   runtime, prompts, capabilities, skills, hooks
  integrity.toml                # generated only — algorithm, hash, chain
  hierarchy.toml                # optional — parent, permitted_children
  capabilities/
    capabilities.toml           # capability-definition.v1 entries
    permissions.toml            # permission-policy half — every capability needs both
    <capability>/handler.ts      # optional TypeScript handler backing
    handlers/tools.json          # generated TypeScript handler manifest
  prompts/*.md                  # persona, terminology, safety, style, …
  package.json                  # TypeScript frontend dependency declaration
  tsconfig.json                 # TypeScript handler type-check configuration
  skills/<skill-id>/            # optional — one shape, compiled or prompt-only
    skill.toml                  #   always present once a skill exists
    SKILL.md, prompt.md         #   author prose / prompt body
    examples/*.air               #   optional worked examples
    skill.air, skill.apxmobj     #   optional compiled artifact; absent = CompileStatus::NotCompiled
  examples/*.md                 # optional agent-level worked examples
  tests/                        # optional agent tests
  shared/                       # optional shared resources
```

`agent.toml` is the only always-required authored file; everything else is
optional but, if present, must match the schema's recognized path patterns —
an unrecognized path fails validation (no agent-private layout).
An entry in `capabilities.toml` with no matching `permissions.toml` entry is
not a capability and fails lint (the "joined capability" rule).

**Integrity chain**: generated `integrity.toml` lists an ordered,
tamper-evident hash chain over recognized files. The chain excludes
`integrity.toml` itself, starts with 64 zeroes as genesis, and its final
link must equal the root `hash`. Reordering, inserting, dropping, or editing
a recognized file breaks the seal. Run `dekk agents agent build`, never hand-patch
`integrity.toml`.

## Decision guide

| Need | Use |
|------|-----|
| Long-lived conversational agent | Declarative `agent.toml` with `[runtime.loop]`, capabilities, and hooks |
| Child agent this package may delegate to | `hierarchy.toml` with `permitted_children` |
| Explicit spawn + delegate in a workflow | `GraphBuilder.spawnAgent(...)` + `GraphBuilder.delegate(...)` |
| Reusable multi-step work | Checked-in `.apxmw` workflow execution |

## Workflow

1. Confirm the agent id, domain, persona, and required capabilities.
2. Author `agent.toml` with `frontend = "typescript"`, a typed runtime loop, prompt paths, capability ids, skills, and hooks.
3. Add prompt fragments under `prompts/`.
4. Add one capability folder per id. Builtins dispatch by id; TypeScript handlers use `handler.ts` and an explicit JSON argument schema.
5. Add one permission entry for every capability. Missing policy is a lint failure.
6. Add local skills under `skills/<id>/` when the behavior is a reusable operating recipe.
7. Run `dekk agents frontend setup` once per checkout, then `dekk agents frontend build`, `dekk agents frontend typecheck-package <agent>/tsconfig.json`, `dekk agents agent sync <agent>`, `dekk agents agent lint <agent>`, and `dekk agents agent build <agent>` to regenerate manifests and the integrity chain.

A skill's prose/prompt files are ordinary agent files under the same
integrity chain as everything else — editing a skill through Gao is editing
its `SKILL.md`/`prompt.md` source, the same way editing any other agent file
is. Landing those edits means re-emitting any compiled skill artifact and
recomputing the hash chain via `dekk agents agent build`.

## Sub-agents

- **In-artifact specialist:** declare the child agent and its flow in the same frontend graph.
- **Graph delegation:** use the typed spawn/delegate operations and explicit capability grants.

## Safety

- Draft agents in chat; applying files or running workers requires write/execute grants.
- Prefer `read_local_skill({"skill_id":"agent-builder"})` when available; otherwise use structured context references to this skill and the first-agent guide.
