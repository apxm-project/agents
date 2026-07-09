# Agent builder

Gao-local skill for creating APXM agents and composing sub-agents.

Use when the operator asks to create an agent, scaffold an agent folder, add a
specialist/sub-agent, or choose between a single agent, in-agent sub-agent,
or workflow delegation.

Canonical reference: `apxm.agent.v1`. The folder contract below is a direct
read of that schema. `agent.toml` is the only authored manifest; generated
`integrity.toml` carries the tamper-evident seal.

## Agent Folder (`apxm.agent.v1`)

```text
<agent-id>/
  agent.toml                    # required — id, version, license, source, compile,
                                 #   runtime, prompts, capabilities, skills, hooks
  integrity.toml                # generated only — algorithm, hash, chain
  hierarchy.toml                # optional — parent, permitted_children
  capabilities/
    capabilities.toml           # capability-definition.v1 entries
    permissions.toml            # permission-policy half — every capability needs both
    handlers/*.py                # optional Python handler backings
  prompts/*.md                  # persona, terminology, safety, style, …
  python/<entry>.py             # required — Python-frontend entry (.py only)
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
a recognized file breaks the seal. Run `apxm agent build`, never hand-patch
`integrity.toml`.

## Decision guide

| Need | Use |
|------|-----|
| Conversational agent with loop, tools, hooks | `ConversationalAgent` |
| Specialist in same artifact | `sub_agents=[Agent(...)]` |
| Explicit spawn + delegate in a workflow | `g.spawn_agent(...)` + `g.delegate(...)` |
| Reusable multi-step work | Checked-in `.apxmw` workflow execution |

## Workflow

1. Confirm agent id, domain, and persona intent.
2. Draft `agent.toml` with `[runtime]`, `[prompts]`, `[capabilities]`, `[chat]`.
3. Add prompt fragments under `prompts/`.
4. Author `python/<entry>.py` with `ConversationalAgent`, validate with `--validate`.
5. Declare capabilities in `capabilities/capabilities.toml` and permissions in `capabilities/permissions.toml` (joined capability rule — both halves or it fails lint).
6. Optionally add local skills under `skills/<id>/` (`skill.toml` + `SKILL.md`/`prompt.md`, prompt-only unless compiled).
7. Run `apxm agent sync`, then `apxm agent lint`, then `apxm agent build` to regenerate aggregate manifests and `integrity.toml`.

A skill's prose/prompt files are ordinary agent files under the same
integrity chain as everything else — editing a skill through Gao is editing
its `SKILL.md`/`prompt.md` source, the same way editing any other agent file
is. Landing those edits means re-emitting any compiled skill artifact and
recomputing the hash chain via `apxm agent build`.

## Sub-agents

- **In-agent:** `sub_agents=[Agent(name="researcher", instructions="...")]` — see `apxm/conversational.py`.
- **Graph delegation:** `g.spawn_agent` + `g.delegate` — see `examples/python/conversational/chat_agent.py`.

## Safety

- Draft agents in chat; applying files or running workers requires write/execute grants.
- Prefer `read_local_skill("agent-builder")` when available; otherwise use structured context references to this skill and the first-agent guide.
