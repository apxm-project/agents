# Runtime Agent Routing

This example uses native AIR to leave an ACP profile unpinned and let the APXM
runtime choose one at execution time.

```bash
dekk apxm agent test codex
dekk apxm execute examples/workflows/agent-routing/runtime_agent_routing.air
```

The `SPAWN_AGENT` result includes the selected `profile` and route explanation
metadata. See [processes.md](../../../docs/pxm/processes.md) for the complete
field list.
