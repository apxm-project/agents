# Agent builder

Gao-local skill for creating APXM agent packages and composing sub-agents.

Use when the operator asks to create an agent, scaffold an agent package, add a
specialist/sub-agent, or choose between a single agent, in-package sub-agent,
or workflow delegation.

Canonical reference: `apxm.agent-package.v1`. The folder contract below is a
direct read of that schema. The APXM agents package-authoring guide has the
most detail on writing the Python entry
with `ConversationalAgent`, but its folder diagram omits
`pack.toml`/`hierarchy.toml`/the integrity chain — follow the schema below for
package *shape*, that guide for entry-code authoring only.

## Package folder (`apxm.agent-package.v1`)

```text
<package-id>/
  pack.toml                     # required — distribution: pack_id, SemVer, license,
                                 #   [source], [integrity] (pack_hash + chain)
  agent.toml                    # required — identity + runtime: id, display_name,
                                 #   kind, domain, entry, [runtime], [prompts],
                                 #   [capabilities], [skills], [chat], [[hooks]]
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
  examples/*.md                 # optional package-level worked examples
  tests/                        # optional package tests
  shared/                       # optional shared resources
```

`pack.toml` and `agent.toml` are the only always-required files; everything
else is optional but, if present, must match the schema's recognized path
patterns — an unrecognized path fails validation (no package-private layout).
An entry in `capabilities.toml` with no matching `permissions.toml` entry is
not a capability and fails lint (the "joined capability" rule).

**Integrity chain**: the package manifest's `files` map lists every recognized
path with its sha256 digest; `integrity.chain` links those digests into an
ordered, tamper-evident hash chain — link 0's `prev_hash` is 64 zeros
(genesis), each later link's `prev_hash` equals the previous link's `hash`,
and the final link's `hash` must equal `integrity.pack_hash`. Reordering,
inserting, or dropping a file breaks the chain. This is why editing a
package's files (including a skill's prose) means re-running the package
build/hash step, not hand-patching the manifest — a stale chain fails
validation even if every individual file digest still looks right.

> Gao's own package (`agents/gao/`) predates the full package layout — it has
> `agent.toml`, `capabilities/`, `prompts/`, `python/`, and prompt-only
> `skills/<id>/` folders (no `skill.toml` yet), but no `pack.toml` or
> integrity chain. Teach operators the schema above for *new* packages
> regardless.

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
7. Draft `pack.toml` (pack_id, SemVer, license, source) once the package is ready to distribute — `id` in the top-level manifest must equal both `pack.pack_id` and `agent.id`.

A skill's prose/prompt files are ordinary package files under the same
integrity chain as everything else — editing a skill through Gao is editing
its `SKILL.md`/`prompt.md` source, the same way editing any other package
file is. Landing those edits means re-emitting any compiled skill artifact and
recomputing the hash chain via the package build step.

## Sub-agents

- **In-package:** `sub_agents=[Agent(name="researcher", instructions="...")]` — see `apxm/conversational.py`.
- **Graph delegation:** `g.spawn_agent` + `g.delegate` — see `examples/python/conversational/chat_agent.py`.

## Safety

- Draft packages in chat; applying files or running workers requires write/execute grants.
- Prefer `gao.read_local_skill("agent-builder")` when available; otherwise use structured context references to this skill and the first-agent guide.
