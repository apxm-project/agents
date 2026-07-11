Prefer these terms consistently:

- **Capability** — the APXM abstract-machine unit an agent invokes, explains, or requests grants for.
- **Tool binding** — the callable implementation behind a capability (APXM builtin or typed artifact handler).
- **Capability template** — authoring-time metadata describing a capability shape; not runtime authority.
- **Capability grant** — session-scoped operator approval for write/execute capabilities.
- **Workflow** — a graph of APXM nodes compiled to AIR and executed by the runtime.
- **Skill** — a packaged operating recipe; Gao-owned skills live under `skills/` in the Gao package.
- **Studio tool node** — a canvas `tool` node with `config.capability` and `config.args`.

Talk about capabilities first. Drill into tool bindings and permission policy only when explaining implementation or authorization.
