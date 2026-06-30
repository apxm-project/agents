"""ConversationalAgent — author the WHOLE conversational agent as one program.

`ConversationalAgent(...).compile()` emits ONE self-contained multi-flow
artifact (a module with several `func.func`): the entry loop flow `main`, the
author turn flow, and one flow per sub-agent, plus the tools and hooks sidecars.
The host (`apxm chat` / the HTTP server) is a dumb pipe — it delivers the user
message and renders streamed tokens; all cognition lives in the program
(constitution #2).

Foundational skeleton: this builds the multi-flow
*shape* and the canonical turn graph, with the reserved named turn parameter
(constitution #6). The native re-arming in-graph loop, the `REGISTER_HOOK`
lowering, and the in-graph compaction subgraph are layered on in
later phases; hooks/compaction/skills are recorded here so they travel with the
artifact and are wired as those phases land.
"""

from __future__ import annotations

import json
from dataclasses import dataclass
from typing import Any

from . import constants as graph_keys
from .agent import Agent
from .constants import DependencyType, ToolGroup, normalize_tool_group
from .hooks import HookFn, hook_descriptor
from .ir import ValidationResult, emit_multi_flow_module, validate_against_apxm
from .proxy import GraphRecorder
from .tools import FunctionTool

# The reserved parameter name the host binds the user message / turn to. Bound by
# NAME, never by position (constitution #6).
TURN_PARAM = "user_message"

# The internal agent name the turn flow is registered under (`<agent>.<flow>`).
_CONVERSATION_AGENT = "conversation"

HOOKS_AIR_COMMENT_PREFIX = "; __apxm_hooks__ "


@dataclass(slots=True)
class CompactionPolicy:
    """Author-owned compaction settings — plain values the USER's program reads.

    APXM enforces NO compaction policy. These fields are consumed by the user's
    own code: `keep_recent` sizes the turn's recall window (the frontend builds
    the recall), and a user `post_turn` hook closes over this object to decide
    WHEN to compact (`compact_at_tokens`, measured via `ctx.count_tokens`), HOW
    (`ctx.ask` with the user's own prompt), and WHERE to store the rolling
    summary (`summary_key`). The frontend pins `summary_key` into the recall so
    whatever the hook stores there is surfaced ahead of the recent window. APXM
    only provides the primitives (`ctx.ask`/`ctx.call`/`ctx.count_tokens`/
    `ctx.recall`/`ctx.umem`); the policy lives entirely in the program.
    """

    keep_recent: int = 4
    compact_at_tokens: int = 20_000
    strategy: str = "summarize"
    summary_key: str = "conversation:summary"


class MultiFlowArtifact:
    """A compiled `ConversationalAgent`: several flows in one module + sidecars."""

    def __init__(
        self,
        graphs: list[Any],
        *,
        python_tools: list[dict[str, Any]] | None = None,
        hooks: list[dict[str, Any]] | None = None,
        metadata: dict[str, Any] | None = None,
    ) -> None:
        self.graphs = graphs
        self.python_tools = python_tools or []
        self.hooks = hooks or []
        self.metadata = metadata or {}

    def to_air(self) -> str:
        """Emit one `module { func.func @main … func.func @<sub>.main }` plus sidecars."""
        air = emit_multi_flow_module(self.graphs)
        prefix = ""
        if self.python_tools:
            manifest = json.dumps(self.python_tools, separators=(",", ":"))
            prefix += f"{graph_keys.PYTHON_TOOLS_AIR_COMMENT_PREFIX}{manifest}\n"
        if self.hooks:
            manifest = json.dumps(self.hooks, separators=(",", ":"))
            prefix += f"{HOOKS_AIR_COMMENT_PREFIX}{manifest}\n"
        return prefix + air

    def validate(self) -> ValidationResult:
        """Validate every flow against the APXM contract; aggregate the result."""
        errors: list[str] = []
        warnings: list[str] = []
        if len(self.graphs) < 2:
            errors.append("expected a multi-flow artifact (entry loop + turn flow)")
        for graph in self.graphs:
            result = validate_against_apxm(graph)
            errors.extend(f"[{graph.name}] {e}" for e in result.errors)
            warnings.extend(f"[{graph.name}] {w}" for w in result.warnings)
        return ValidationResult(valid=len(errors) == 0, errors=errors, warnings=warnings)

    @property
    def flow_names(self) -> list[str]:
        return [g.name for g in self.graphs]


class ConversationalAgent:
    """Declarative whole-agent builder. See `contracts/python-api.md`."""

    def __init__(
        self,
        *,
        persona: str,
        memory_space: str = "stm",
        tools: list[FunctionTool] | None = None,
        tool_groups: list[ToolGroup | str] | None = None,
        skills: bool = False,
        sub_agents: list[Agent] | None = None,
        compaction: CompactionPolicy | None = None,
        hooks: list[HookFn] | None = None,
        loop: str = "in_graph",
    ) -> None:
        if loop not in ("host", "in_graph"):
            raise ValueError(f"loop must be 'host' or 'in_graph', got {loop!r}")
        self.persona = persona
        self.memory_space = memory_space
        self.tools = list(tools or [])
        self.tool_groups = [normalize_tool_group(group) for group in (tool_groups or [])]
        self.skills = skills
        self.sub_agents = list(sub_agents or [])
        self.compaction = compaction
        self.hooks = list(hooks or [])
        self.loop = loop

        for t in self.tools:
            if not isinstance(t, FunctionTool):
                raise TypeError("tools must be @tool-decorated FunctionTool objects")
        for h in self.hooks:
            if not isinstance(h, HookFn):
                raise TypeError("hooks must be @hook-decorated HookFn objects")
        for a in self.sub_agents:
            if not isinstance(a, Agent):
                raise TypeError("sub_agents must be Agent objects")

        # Build-time delegate target validation: sub-agent
        # names must be unique so `<name>.main` flows do not collide.
        names = [a.name for a in self.sub_agents]
        dupes = {n for n in names if names.count(n) > 1}
        if dupes:
            raise ValueError(f"duplicate sub_agent name(s): {sorted(dupes)}")

    def compile(self) -> MultiFlowArtifact:
        turn = self._build_turn_flow()
        entry = self._build_entry_flow()
        graphs = [entry.to_graph(), turn.to_graph()]

        python_tools: list[dict[str, Any]] = list(turn._python_tools)
        seen_ids = {d.get(graph_keys.PYTHON_TOOL_MANIFEST_HANDLER_ID) for d in python_tools}

        for sub in self.sub_agents:
            sub_rec = self._build_sub_agent_flow(sub)
            graphs.append(sub_rec.to_graph())
            for desc in sub_rec._python_tools:
                if desc.get(graph_keys.PYTHON_TOOL_MANIFEST_HANDLER_ID) not in seen_ids:
                    python_tools.append(desc)
                    seen_ids.add(desc.get(graph_keys.PYTHON_TOOL_MANIFEST_HANDLER_ID))

        hooks_sidecar = [hook_descriptor(h) for h in self.hooks]

        # Hooks share the @tool invocation path: add a tool-bridge manifest entry
        # for each hook handler so the runtime `PythonToolBridge` can resolve and
        # dispatch it by handler_id (constitution #4 — one Python-handler path).
        import inspect as _inspect

        for h in self.hooks:
            if h.handler_id in seen_ids:
                continue
            module = getattr(h.fn, "__module__", "__unknown__") or "__unknown__"
            qualname = getattr(h.fn, "__qualname__", h.name)
            desc = {
                graph_keys.PYTHON_TOOL_MANIFEST_HANDLER_ID: h.handler_id,
                graph_keys.PYTHON_TOOL_MANIFEST_MODULE: module,
                graph_keys.PYTHON_TOOL_MANIFEST_QUALNAME: qualname,
                graph_keys.PYTHON_TOOL_MANIFEST_NAME: h.name,
                graph_keys.PYTHON_TOOL_MANIFEST_DESCRIPTION: f"lifecycle hook ({h.event})",
                graph_keys.PYTHON_TOOL_MANIFEST_SCHEMA: {},
            }
            src = _inspect.getsourcefile(h.fn)
            if src:
                desc[graph_keys.PYTHON_TOOL_MANIFEST_SOURCE_FILE] = src
            python_tools.append(desc)
            seen_ids.add(h.handler_id)

        metadata = {
            "loop": self.loop,
            "memory_space": self.memory_space,
            "skills": self.skills,
            "tool_groups": self.tool_groups,
            "turn_param": TURN_PARAM,
        }
        if self.compaction is not None:
            metadata["compaction"] = {
                "keep_recent": self.compaction.keep_recent,
                "compact_at_tokens": self.compaction.compact_at_tokens,
                "strategy": self.compaction.strategy,
            }

        return MultiFlowArtifact(
            graphs,
            python_tools=python_tools,
            hooks=hooks_sidecar,
            metadata=metadata,
        )

    # -- flow builders --------------------------------------------------------

    def _build_turn_flow(self) -> GraphRecorder:
        """The author turn body: recall → ask(tools) → remember → done."""
        turn = GraphRecorder(
            f"{_CONVERSATION_AGENT}.turn",
            metadata={graph_keys.IS_ENTRY: False},
        )
        turn.param(TURN_PARAM, "str")

        # Recall the recent transcript window — the real cross-turn memory the
        # runtime records: assistant answers under `conversation:turn:<n>`
        # (ConversationMemoryMiddleware) and user messages under
        # `conversation:user:<n>` (the turn-input endpoint). The shared
        # `conversation:` prefix reads BOTH sides in temporal order (M2/M3),
        # not a relevance search over a constant key.
        keep_recent = self.compaction.keep_recent if self.compaction else 4
        # Pin the author's rolling-summary key so whatever a compaction hook
        # stores there is surfaced ahead of the recent window (it has no numeric
        # transcript order and would otherwise drop out). WHICH key is the
        # program's choice (CompactionPolicy.summary_key), not a runtime default.
        recall_kwargs: dict[str, Any] = {}
        if self.compaction is not None:
            recall_kwargs["recall_pin"] = self.compaction.summary_key
        history = turn.query_memory(
            name="recall",
            recall_mode="recent",
            recent=max(1, keep_recent) * 2,  # ~keep_recent turns (user+assistant)
            recall_prefix="conversation:",
            space=self.memory_space,
            **recall_kwargs,
        )

        # Register author tools so they are runtime-registered + travel in the
        # tools sidecar; ask then advertises them by name.
        registrations = [turn.register_tool(t) for t in self.tools]

        ask_attrs: dict[str, Any] = {}
        # Mark this ask as the top-level conversational turn. The runtime's
        # ConversationMemoryMiddleware scopes turn accounting + lifecycle hooks
        # (pre_turn/post_turn/post_ask) to the marked turn, so sub-agent asks
        # (which share this session's memory scope) do not inflate the turn
        # count, pollute the recall window, or re-fire turn hooks.
        ask_attrs["conversational_turn"] = "true"
        tool_names = [t.name for t in self.tools]
        if tool_names:
            ask_attrs[graph_keys.TOOLS] = tool_names
        # `skills=True` wires the real description-based discovery group
        # (`search_skills`) into the turn so the agent can pick a skill by
        # description (constitution: real capability, no stub).
        groups = list(self.tool_groups)
        if self.skills:
            groups.append(ToolGroup.SKILLS.value)
        if groups:
            ask_attrs["tool_groups"] = groups

        answer = turn.ask(
            name="answer",
            prompt="{user_message}\n\nRecalled context:\n{history}",
            system_prompt=self.persona,
            **ask_attrs,
        )
        for reg in registrations:
            turn.add_edge(reg, answer, dependency=DependencyType.CONTROL)

        # Cross-turn memory is genuine and automatic: the runtime records the
        # assistant answer per turn (ConversationMemoryMiddleware) and the user
        # message at the turn-input endpoint, both ordered + session-scoped, so
        # the `recall` above reads the real transcript. The author can ALSO write
        # a deliberate summary via a `post_turn` hook (`ctx.umem(...)`). No
        # constant-literal placeholder write here (M3).
        turn.done(source=answer)
        _ = history  # referenced via the {history} template above
        return turn

    def _build_entry_flow(self) -> GraphRecorder:
        """The conversation loop: park for a turn input, run the turn flow.

        Foundational skeleton: emits the loop *entry* node (an AUTONOMOUS recv,
        re-arming) and a flow-call into the turn body. The native park + re-arm
        (no node re-execution) is the keystone; this shape validates and is
        the seam the runtime loop wires onto.
        """
        entry = GraphRecorder("main", metadata={graph_keys.IS_ENTRY: True})
        entry.param(TURN_PARAM, "str")

        # Lower each author hook to a REGISTER_HOOK node so the binding travels
        # inside the artifact (AIR-portable) and installs into the runtime hook
        # registry before the loop runs. The handler is dispatched via the shared
        # tool bridge (constitution #4); its manifest entry is added in compile().
        hook_nodes = []
        for h in self.hooks:
            hook_nodes.append(
                entry.register_hook(
                    event=h.event,
                    handler_id=h.handler_id,
                    match=h.match,
                    mode=h.mode,
                )
            )

        if self.loop == "in_graph":
            # The recv node is the native loop anchor AND the entry's exit. It
            # parks on the session turn-input key; on each wake the runtime
            # splices a fresh turn flow-call (binding the user message to the
            # reserved `turn_param`) + a fresh recv (re-arm) — see
            # `scheduler::rearm_session_turn`. The turn is therefore NOT statically
            # called here (that would double-run it alongside the spliced call);
            # the `turn_*` attrs tell the runtime which flow to dispatch and how to
            # bind the message (constitution #6).
            loop_node = entry.autonomous(
                name="turn_loop",
                prompt=self.persona,
                mode="recv",
                recv_once="false",
                turn_agent=_CONVERSATION_AGENT,
                turn_flow="turn",
                turn_param=TURN_PARAM,
            )
            # Hooks register before the loop arms (session_start can gate); the
            # recv anchor is the func exit (it re-arms for the session's life).
            for hn in hook_nodes:
                entry.add_edge(hn, loop_node, dependency=DependencyType.CONTROL)
            _ = loop_node
        else:
            # loop="host": the thin host owns the outer loop; the entry IS the
            # turn body invocation.
            run_turn = entry.flow_call(
                name="run_turn",
                agent_name=_CONVERSATION_AGENT,
                flow_name="turn",
            )
            for hn in hook_nodes:
                entry.add_edge(hn, run_turn, dependency=DependencyType.CONTROL)
            entry.done(source=run_turn)

        return entry

    def _build_sub_agent_flow(self, sub: Agent) -> GraphRecorder:
        """One `<name>.main` flow per sub-agent, resolved in the SAME artifact."""
        rec = GraphRecorder(
            f"{sub.name}.main",
            metadata={graph_keys.IS_ENTRY: False},
        )
        rec.param(TURN_PARAM, "str")

        registrations = [rec.register_tool(t) for t in sub._tools]
        ask_attrs: dict[str, Any] = {}
        tool_names = [t.name for t in sub._tools]
        if tool_names:
            ask_attrs[graph_keys.TOOLS] = tool_names

        answer = rec.ask(
            name="answer",
            prompt="{user_message}",
            system_prompt=sub.instructions or "",
            **ask_attrs,
        )
        for reg in registrations:
            rec.add_edge(reg, answer, dependency=DependencyType.CONTROL)
        rec.done(source=answer)
        return rec


__all__ = ["CompactionPolicy", "ConversationalAgent", "MultiFlowArtifact", "TURN_PARAM"]
