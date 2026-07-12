"""Ergonomic sugar for common APXM workflow patterns."""

from __future__ import annotations

import inspect
from typing import TYPE_CHECKING, Any, Mapping

from . import constants as graph_keys
from ._generated.operations import ASK, REASON, THINK, OpSpec
from .proxy import GraphRecorder, NodeRef

if TYPE_CHECKING:
    from ._generated.agents import AgentRef
    from ._generated.models import ModelId

_AGENT_ASK_OPERATION = ASK
_AGENT_THINK_OPERATION = THINK
_AGENT_REASON_OPERATION = REASON
_AGENT_MESSAGE_NODE_SEGMENT = "msg"


class AgentHandle:
    """Handle to a spawned agent that emits COMMUNICATE nodes."""

    def __init__(self, recorder: GraphRecorder, spawn_node: NodeRef, agent_name: str) -> None:
        self._recorder = recorder
        self._spawn_node = spawn_node
        self._agent_name = agent_name
        self._last_node = spawn_node
        self._msg_counter = 0

    def ask(
        self,
        message: str | None = None,
        *,
        prompt: str | None = None,
        **attributes: Any,
    ) -> NodeRef:
        """Send a message to the spawned agent via COMMUNICATE node.

        Returns the COMMUNICATE node.

        The message can contain {var_name} references which will be auto-wired
        to NodeRef variables in the caller's scope.
        """
        if message is None and prompt is None:
            raise ValueError("ask() missing required message")
        if message is not None and prompt is not None:
            raise ValueError("ask() accepts either message or prompt, not both")
        message_text = message if message is not None else prompt
        assert message_text is not None

        return self._turn(
            message_text,
            llm_operation=_AGENT_ASK_OPERATION,
            template_scope=_caller_locals(),
            **attributes,
        )

    def think(
        self,
        message: str | None = None,
        *,
        prompt: str | None = None,
        **attributes: Any,
    ) -> NodeRef:
        if message is None and prompt is None:
            raise ValueError("think() missing required message")
        if message is not None and prompt is not None:
            raise ValueError("think() accepts either message or prompt, not both")
        message_text = message if message is not None else prompt
        assert message_text is not None

        return self._turn(
            message_text,
            llm_operation=_AGENT_THINK_OPERATION,
            template_scope=_caller_locals(),
            **attributes,
        )

    def reason(
        self,
        message: str | None = None,
        *,
        prompt: str | None = None,
        **attributes: Any,
    ) -> NodeRef:
        if message is None and prompt is None:
            raise ValueError("reason() missing required message")
        if message is not None and prompt is not None:
            raise ValueError("reason() accepts either message or prompt, not both")
        message_text = message if message is not None else prompt
        assert message_text is not None

        return self._turn(
            message_text,
            llm_operation=_AGENT_REASON_OPERATION,
            template_scope=_caller_locals(),
            **attributes,
        )

    def chat(self, turns: list[str], **attributes: Any) -> NodeRef:
        """Emit a fixed multi-turn conversation script against this agent.

        Each entry in ``turns`` becomes a COMMUNICATE turn that takes a Data
        dependency on the previous turn, producing a sequential exchange with
        one spawned agent. Template refs (``{name}``) auto-wire from the
        caller's scope. Returns the final turn's NodeRef.

        This is the in-graph helper for a *bounded* conversation script. An
        open-ended interactive conversation is driven from the host instead
        (see ``apxm chat``), re-running the graph once per user message.
        """
        if not turns:
            raise ValueError("chat() requires at least one turn")
        scope = _caller_locals()
        last: NodeRef | None = None
        for msg in turns:
            node = self._turn(
                msg,
                llm_operation=_AGENT_ASK_OPERATION,
                template_scope=scope,
                **attributes,
            )
            if last is not None:
                self._recorder.add_edge(
                    last, node, dependency=graph_keys.DEPENDENCY_DATA
                )
            last = node
        assert last is not None
        return last

    def _turn(
        self,
        message: str,
        *,
        llm_operation: OpSpec,
        template_scope: Mapping[str, Any],
        **attributes: Any,
    ) -> NodeRef:
        self._msg_counter += 1
        node_name = f"{self._agent_name}_{_AGENT_MESSAGE_NODE_SEGMENT}_{self._msg_counter}"
        turn_attributes = {graph_keys.LLM_OPERATION: llm_operation.op, **attributes}

        comm_node = self._recorder.communicate(
            name=node_name,
            target_agent=self._agent_name,
            message=message,
            protocol=graph_keys.COMMUNICATE_PROTOCOL_ACP,
            _template_scope=template_scope,
            **turn_attributes,
        )

        self._last_node = comm_node
        return comm_node

    def get_spawn_node(self) -> NodeRef:
        """Return the initial SPAWN_AGENT node."""
        return self._spawn_node


def _caller_locals() -> Mapping[str, Any]:
    caller_frame = inspect.currentframe()
    if caller_frame is None:
        return {}
    try:
        if caller_frame.f_back and caller_frame.f_back.f_back:
            return dict(caller_frame.f_back.f_back.f_locals)
        return {}
    finally:
        del caller_frame


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


def spawn_agent_set(
    g: GraphRecorder,
    candidates: list[tuple[str, str]] | list[str],
    *,
    scope_policy: str = "Snapshot",
    body,
) -> list[NodeRef]:
    """Fan out across a compile-time list of candidate agents.

    Each candidate spawns in parallel under `scope_policy`. Body is
    callable(handle, slot_key, agent_name) returning a NodeRef result
    suitable for wait_all/merge.

    `candidates` is either a list of (slot_key, agent_name) tuples or a
    flat list of agent names (slot_key defaults to a sanitized agent name).

    The candidate list bounds the fan-out at compile time. Runtime
    selection happens inside `body` — e.g. the body can pass `plan` as a
    runtime dep so the runtime no-ops slots not present in the plan.
    """
    if not candidates:
        raise ValueError("spawn_agent_set requires at least one candidate")
    if not callable(body):
        raise TypeError("body must be callable(handle, slot_key, agent_name)")
    normalized: list[tuple[str, str]] = []
    for entry in candidates:
        if isinstance(entry, str):
            slot = entry.replace(".", "_").replace("-", "_")
            normalized.append((slot, entry))
        else:
            normalized.append((entry[0], entry[1]))
    results: list[NodeRef] = []
    for slot_key, agent_name in normalized:
        handle = g.spawn(
            agent_name,
            **{graph_keys.SCOPE_POLICY_KEY: scope_policy}
            if hasattr(graph_keys, "SCOPE_POLICY_KEY")
            else {"scope_policy": scope_policy},
        )
        results.append(body(handle, slot_key, agent_name))
    return results


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


# There is no `Loop`/`g.loop()` sugar. The prior version wrapped
# LOOP_START/LOOP_END, which compiled and verified but never re-executed at
# runtime (the executor is a DAG engine with no back-edge or re-splice wired
# to either handler) — a fire-once IR lie. Both ops were deleted from the
# catalog rather than kept as compiled-but-ignored ops. For real in-graph
# iteration use the host turn-loop (``apxm chat``, which drives the splice-
# based session/turn re-arm mechanism) or ``AUTONOMOUS`` for a fused
# plan/act/evaluate macro-op. In-graph iteration uses graph splicing.


# Monkey-patch GraphRecorder to add ergonomic methods
GraphRecorder.spawn = _spawn_with_handle  # type: ignore[assignment]
GraphRecorder.team = _create_team  # type: ignore[assignment]


__all__ = ["AgentHandle", "Team"]
