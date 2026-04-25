# Multi-Agent

Native multi-agent coordination with spawn, communicate, and teams.

## Why This Matters

APXM has first-class primitives for multi-agent workflows. Agents are spawned
as ACP subprocesses, communicate via typed messages, and can be organized into
teams with barrier synchronization and result merging.

## Requirements

These examples import generated ACP profiles from `apxm._generated.agents`.
Use `dekk apxm agent list` to confirm the profiles exist and
`dekk apxm agent test <name>` before executing a graph. The checked-in
`claude` and `codex` profiles require Node/npm and their respective
authenticated CLIs.

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
result = agent.ask("Implement this feature")

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
