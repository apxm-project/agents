# Runtime Agent Routing

This example uses native AIR to leave an ACP profile unpinned and let the APXM
runtime choose one at execution time.

```bash
dekk apxm agent test codex
dekk apxm execute examples/workflows/agent-routing/runtime_agent_routing.air
```

The `SPAWN_AGENT` result includes the selected `profile`, `route_source`,
`route_action`, `route_policy`, `route_reason`, `route_scores`,
`eligible_profiles`, and `rejected_profiles`.
