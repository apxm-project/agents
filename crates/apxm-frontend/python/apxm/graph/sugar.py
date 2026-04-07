"""Ergonomic sugar for common APXM workflow patterns."""

from __future__ import annotations

from typing import Any

from apxm._generated import constants as c
from . import constants as graph_keys
from .proxy import GraphRecorder, NodeRef


class AgentHandle:
    """Handle to a spawned agent that supports method chaining for COMMUNICATE nodes."""

    def __init__(self, recorder: GraphRecorder, spawn_node: NodeRef, agent_name: str) -> None:
        self._recorder = recorder
        self._spawn_node = spawn_node
        self._agent_name = agent_name
        self._last_node = spawn_node
        self._msg_counter = 0

    def ask(self, message: str, **attributes: Any) -> "AgentHandle":
        """Send a message to the spawned agent via COMMUNICATE node.

        Returns self for method chaining.
        """
        self._msg_counter += 1
        node_name = f"{self._agent_name}_msg_{self._msg_counter}"

        comm_node = self._recorder._add_node(
            node_name,
            graph_keys.OP_COMMUNICATE,
            {
                c.RECIPIENT: self._agent_name,
                c.MESSAGE: message,
                **attributes,
            },
        )

        # Create control edge from previous node to this communicate node
        self._last_node >> comm_node

        self._last_node = comm_node
        return self

    def get_spawn_node(self) -> NodeRef:
        """Return the initial SPAWN_AGENT node."""
        return self._spawn_node

    def get_last_node(self) -> NodeRef:
        """Return the last node in the agent communication chain."""
        return self._last_node


class Team:
    """Workflow-local team of agents.

    Provides sugar for spawning multiple agents and collecting their results.
    """

    def __init__(self, recorder: GraphRecorder, name: str) -> None:
        self._recorder = recorder
        self._name = name
        self._members: list[AgentHandle] = []

    def add(
        self,
        agent_name: str,
        profile: str | None = None,
        mode: str | None = None,
        model: str | None = None,
        cwd: str | None = None,
        **attributes: Any,
    ) -> AgentHandle:
        """Add an agent to the team by spawning it.

        Returns an AgentHandle for method chaining.
        """
        member_index = len(self._members)
        spawn_name = f"{self._name}_{agent_name}_{member_index}"

        spawn_node = self._recorder.spawn_agent(
            spawn_name,
            agent_name=agent_name,
            profile=profile,
            mode=mode,
            model=model,
            cwd=cwd,
            **attributes,
        )

        handle = AgentHandle(self._recorder, spawn_node, agent_name)
        self._members.append(handle)
        return handle

    def wait_all(self, name: str | None = None) -> NodeRef:
        """Create a WAIT_ALL node that waits for all team members to complete.

        Returns the WAIT_ALL NodeRef.
        """
        if not self._members:
            raise ValueError(f"team '{self._name}' has no members")

        wait_name = name or f"{self._name}_wait_all"
        last_nodes = [member.get_last_node() for member in self._members]
        return self._recorder.wait_all(wait_name, last_nodes)

    def merge(self, name: str | None = None) -> NodeRef:
        """Create a MERGE node that merges outputs from all team members.

        Returns the MERGE NodeRef.
        """
        if not self._members:
            raise ValueError(f"team '{self._name}' has no members")

        merge_name = name or f"{self._name}_merge"
        last_nodes = [member.get_last_node() for member in self._members]
        return self._recorder.merge(merge_name, last_nodes)


# Extend GraphRecorder with ergonomic spawn() method
def _spawn_with_handle(
    self: GraphRecorder,
    agent_name: str,
    profile: str | None = None,
    mode: str | None = None,
    model: str | None = None,
    cwd: str | None = None,
    **attributes: Any,
) -> AgentHandle:
    """Spawn an agent and return an AgentHandle for method chaining.

    This is sugar over spawn_agent() that returns an AgentHandle instead of NodeRef.
    """
    spawn_node = self.spawn_agent(
        agent_name,
        agent_name=agent_name,
        profile=profile,
        mode=mode,
        model=model,
        cwd=cwd,
        **attributes,
    )
    return AgentHandle(self, spawn_node, agent_name)


def _create_team(self: GraphRecorder, name: str) -> Team:
    """Create a workflow-local team of agents.

    Example:
        team = g.team("research_team")
        alice = team.add("alice", profile="claude")
        bob = team.add("bob", profile="codex")
        alice.ask("Research X")
        bob.ask("Research Y")
        results = team.merge()
    """
    return Team(self, name)


# Monkey-patch GraphRecorder to add ergonomic methods
GraphRecorder.spawn = _spawn_with_handle  # type: ignore[assignment]
GraphRecorder.team = _create_team  # type: ignore[assignment]


__all__ = ["AgentHandle", "Team"]
