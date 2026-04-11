# Multi-Agent

Native multi-agent coordination with spawn, communicate, and teams.

## Why This Matters

APXM has first-class primitives for multi-agent workflows. Agents are spawned
as ACP subprocesses, communicate via typed messages, and can be organized into
teams with barrier synchronization and result merging.

## Examples

- **spawn_and_communicate.py** -- Basic spawn + communicate, then two-agent pipeline. `dekk apxm execute examples/python/multi-agent/spawn_and_communicate.py`
- **parallel_agents.py** -- Multiple agents working in parallel. `dekk apxm execute examples/python/multi-agent/parallel_agents.py`
- **negotiation.py** -- Multi-agent negotiation to consensus. `dekk apxm execute examples/python/multi-agent/negotiation.py`
- **team_coordination.py** -- Team sugar: g.team(), add(), wait_all(), merge(). `dekk apxm execute examples/python/multi-agent/team_coordination.py`

## Key API

```python
from apxm._generated.agents import claude, codex

# Spawn and communicate
agent = g.spawn("coder", profile=codex, cwd=cwd)
agent.ask("Implement this feature")
result = agent.get_last_node()

# Team coordination
team = g.team("dev_team")
arch = team.add("architect", profile=claude)
coder = team.add("coder", profile=codex)
arch.ask("Design the API")
coder.ask("Write the server")
team.merge("results")
```

## Learn More

- [Sugar API](../../../crates/compiler/apxm-frontend/python/apxm/sugar.py)
- [Agent profiles](../../../crates/compiler/apxm-frontend/python/apxm/_generated/agents.py)
