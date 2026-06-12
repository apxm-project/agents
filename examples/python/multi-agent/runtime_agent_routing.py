#!/usr/bin/env python3
"""runtime_agent_routing.py -- APXM runtime profile selection for SPAWN_AGENT.

This example leaves the concrete ACP profile unpinned. The runtime chooses from
host-discovered route candidates using required_capabilities and
preferred_profiles, then SPAWN_AGENT returns the selected route metadata.

Usage:
    dekk apxm agent test codex
    dekk apxm execute examples/python/multi-agent/runtime_agent_routing.py
"""

from apxm import GraphRecorder, agent_cwd, compile


@compile()
def runtime_agent_routing(g: GraphRecorder):
    """Route one worker through the runtime AgentRouter, then return metadata."""
    routed_worker = g.spawn_agent(
        "spawn_routed_worker",
        agent_name="routed_worker",
        agent_route="auto",
        required_capabilities=["execute"],
        preferred_profiles=["codex"],
        cwd=agent_cwd(),
    )
    g.done(routed_worker)


if __name__ == "__main__":
    import apxm

    result = apxm.run(runtime_agent_routing())
    print(result.content)
