"""Ergonomic sugar for common APXM workflow patterns."""

from __future__ import annotations

from typing import TYPE_CHECKING, Any

from apxm._generated import constants as c
from . import constants as graph_keys
from .proxy import GraphRecorder, NodeRef

if TYPE_CHECKING:
    from ._generated.agents import AgentRef
    from ._generated.models import ModelId


class AgentHandle:
    """Handle to a spawned agent that emits COMMUNICATE nodes."""

    def __init__(self, recorder: GraphRecorder, spawn_node: NodeRef, agent_name: str) -> None:
        self._recorder = recorder
        self._spawn_node = spawn_node
        self._agent_name = agent_name
        self._last_node = spawn_node
        self._msg_counter = 0

    def ask(self, message: str, **attributes: Any) -> NodeRef:
        """Send a message to the spawned agent via COMMUNICATE node.

        Returns the COMMUNICATE node.

        The message can contain {var_name} references which will be auto-wired
        to NodeRef variables in the caller's scope.
        """
        self._msg_counter += 1
        node_name = f"{self._agent_name}_msg_{self._msg_counter}"

        # Auto-wire: resolve {var_name} to NodeRef
        resolved_message, auto_refs = self._recorder._resolve_template_refs(message)

        attrs = {
            c.RECIPIENT: self._agent_name,
            c.MESSAGE: resolved_message,
            c.PROTOCOL: graph_keys.COMMUNICATE_PROTOCOL_ACP,
            **attributes,
        }
        if auto_refs:
            attrs[graph_keys.INPUT_NAMES] = [name for name, _ in auto_refs]

        comm_node = self._recorder._add_node(node_name, graph_keys.OP_COMMUNICATE, attrs)

        # Create control edge from previous node to this communicate node
        self._recorder.add_edge(
            self._last_node,
            comm_node,
            dependency=graph_keys.DEPENDENCY_CONTROL,
        )

        # Create auto-wire data edges
        for _name, ref in auto_refs:
            self._recorder.add_edge(
                ref,
                comm_node,
                dependency=graph_keys.DEPENDENCY_DATA,
            )

        self._last_node = comm_node
        return comm_node

    def get_spawn_node(self) -> NodeRef:
        """Return the initial SPAWN_AGENT node."""
        return self._spawn_node

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
        profile: AgentRef | None = None,
        mode: str | None = None,
        model: ModelId | None = None,
        cwd: str | None = None,
        **attributes: Any,
    ) -> AgentHandle:
        """Add an agent to the team by spawning it.

        Args:
            agent_name: Name of the agent instance
            profile: Agent profile from apxm._generated.agents
            mode: Agent mode
            model: Model name
            cwd: Working directory

        Returns an AgentHandle for sending messages to the spawned agent.
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
        last_nodes = [member._last_node for member in self._members]
        return self._recorder.wait_all(wait_name, last_nodes)

    def merge(self, name: str | None = None) -> NodeRef:
        """Create a MERGE node that merges outputs from all team members.

        Returns the MERGE NodeRef.
        """
        if not self._members:
            raise ValueError(f"team '{self._name}' has no members")

        merge_name = name or f"{self._name}_merge"
        last_nodes = [member._last_node for member in self._members]
        return self._recorder.merge(merge_name, last_nodes)


# Extend GraphRecorder with ergonomic spawn() method
def _spawn_with_handle(
    self: GraphRecorder,
    agent_name: str,
    profile: AgentRef | None = None,
    mode: str | None = None,
    model: ModelId | None = None,
    cwd: str | None = None,
    **attributes: Any,
) -> AgentHandle:
    """Spawn an agent and return an AgentHandle.

    This is sugar over spawn_agent() that returns an AgentHandle instead of NodeRef.

    Args:
        agent_name: Name of the agent instance
        profile: Agent profile from apxm._generated.agents
        mode: Agent mode
        model: Model name
        cwd: Working directory
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
        from apxm._generated.agents import claude, codex

        team = g.team("research_team")
        alice = team.add("alice", profile=claude)
        bob = team.add("bob", profile=codex)
        alice.ask("Research X")
        bob.ask("Research Y")
        results = team.merge()
    """
    return Team(self, name)


# Monkey-patch GraphRecorder to add ergonomic methods
GraphRecorder.spawn = _spawn_with_handle  # type: ignore[assignment]
GraphRecorder.team = _create_team  # type: ignore[assignment]


__all__ = ["AgentHandle", "Team"]
