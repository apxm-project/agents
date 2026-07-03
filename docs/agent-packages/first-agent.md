# Create your first APXM agent package

An APXM **agent package** is one folder with an `agent.toml` index and three
required content areas. Everything else is optional support.

```text
<agent-id>/
  agent.toml
  prompts/
  python/<agent-entry>.py
  capabilities/
```

Optional folders such as `README.md`, `examples/`, `tests/`, or `skills/` can
help operators and maintainers, but they are **not** part of the minimum contract.

## Minimum `agent.toml`

Studio and package loaders expect these fields (see
`workspace/studio/crates/studio/src/agents.rs`):

| Field | Meaning |
|-------|---------|
| `id` | Stable agent id (`gao`) |
| `display_name` | Human label |
| `kind` | Package kind (`conversational`) |
| `domain` | Domain tag |
| `description` | Short summary |
| `entry` | Relative path to Python entry |

Tables:

- `[runtime]` — `loop`, `memory_space`, optional `session_prefix`
- `[prompts]` — logical name → markdown path under `prompts/`
- `[capabilities]` — paths to `capabilities/capabilities.toml` and `permissions.toml`
- `[chat]` — Studio chat flags: `capability_discovery`, `skills`, `authoring`

Optional `[skills].local` lists agent-owned skill directories (not required).

## Python entry

Author the agent with `ConversationalAgent` from the APXM Python frontend
(`crates/compiler/frontend/python/apxm/conversational.py`):

```python
#!/usr/bin/env python3
from apxm import Agent, ConversationalAgent, ToolGroup, tool

@tool
def ping() -> str:
    """Health check."""
    return "pong"

researcher = Agent(name="researcher", instructions="Gather supporting facts.")

agent = ConversationalAgent(
    persona="You are a helpful APXM assistant.",
    memory_space="stm",
    tools=[ping],
    capability_groups=[ToolGroup.SKILLS],
    sub_agents=[researcher],
    loop="in_graph",
)

main = agent.compile()

if __name__ == "__main__":
    import sys
    if "--validate" in sys.argv:
        result = main.validate()
        print("VALID" if result.valid else "INVALID")
        for err in result.errors:
            print(f"  ERROR: {err}")
        sys.exit(0 if result.valid else 1)
    print(main.to_air())
```

Validate and emit AIR:

```bash
PYTHONPATH=crates/compiler/frontend/python python3 <agent-id>/python/<entry>.py --validate
PYTHONPATH=crates/compiler/frontend/python python3 <agent-id>/python/<entry>.py
```

## Capabilities

Declare capabilities in `capabilities/capabilities.toml`:

- **Read-only** capabilities (`read_only = true`) — discovery, planning, local skill reads
- **Write / execute** capabilities — require operator grants at runtime (`effect = "ask"`)

Runtime grants are **session-scoped**. Declaring a capability in TOML does not
grant authority; the operator must approve write-class tools per conversation or
workflow.

Reference bindings:

- `kind = "python_handler"` — local Python function in the package
- `kind = "apxm_builtin"` — runtime group such as `skills` or `authoring`

## Sub-agents

Three shipped patterns (all valid; pick by scope):

| Pattern | When to use | Source |
|---------|-------------|--------|
| `sub_agents=[Agent(...)]` | Specialists compiled into the same artifact | `apxm/conversational.py` |
| `g.spawn_agent` + `g.delegate` | Explicit delegation steps in a workflow graph | `examples/python/conversational/chat_agent.py` |
| APXM goal orchestration | Bounded worker fan-out/fan-in with server-owned lifecycle | `.agents/skills/goal-orchestrator/SKILL.md` |

## Optional support

Add when useful:

- **`skills/<id>/SKILL.md`** — agent-owned recipes (Gao uses this for workflow design)
- **`examples/`** — operator-facing samples
- **`tests/`** — package structure and prompt validation
- **`README.md`** — package-specific notes

## Studio integration

Studio discovers bundled packages under `workspace/studio/agents/<id>/`:

1. `$APXM_STUDIO_AGENTS` when set
2. Bundled `agents/` next to the studio crate
3. `$APXM_WORKSPACE_ROOT/agents` when present

Chat selects an agent with `agent_preset: "<id>"`. For bundled conversational
packages such as Gao, Studio compiles the Python entry (`main.to_air()`) and
dispatches that artifact through the same `/v1/execute/stream` path as canvas
workflows, including python tool sidecars. Other presets may still use generic
`chat_air()` built from `[chat]` flags until they adopt the same compile path.

See also: [Studio agent packages](../../../studio/agents/README.md).
