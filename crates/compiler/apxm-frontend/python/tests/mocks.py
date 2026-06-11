"""Typed test fixtures for Python frontend graph construction."""

from __future__ import annotations

from apxm._generated.agents import AgentRef

MOCK_AGENT_PROFILE = AgentRef(
    name="mock-agent-profile",
    command="mock-agent --stdio",
    description="Mock agent profile for frontend tests.",
    route_capabilities=("read", "write", "execute", "critique", "workflow_author"),
    source="test",
    default_mode=None,
    default_model=None,
)
MOCK_AGENT_PROFILE_ALT = AgentRef(
    name="mock-agent-profile-alt",
    command="mock-agent-alt --stdio",
    description="Alternate mock agent profile for frontend tests.",
    route_capabilities=("read", "write", "execute", "critique", "workflow_author"),
    source="test",
    default_mode=None,
    default_model=None,
)
