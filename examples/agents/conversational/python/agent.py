"""A model-directed conversational Agent Program on the typed frontend.

The authored loop owns persistent conversation state, typed Tool dispatch,
Model re-entry, context-management Hooks, and the yield/resume boundary. No
conversation-specific runtime or hidden Model/Tool loop supplies these
semantics.
"""

from __future__ import annotations

import json
import sys
from dataclasses import dataclass
from typing import Literal

from apxm_program import Agent, Context, Hook, Model, Tool


@dataclass(frozen=True, slots=True)
class ConversationInput:
    message: str


@dataclass(frozen=True, slots=True)
class ConversationOutput:
    message: str


ToolKind = Literal["search_web", "read_page"]


@dataclass(frozen=True, slots=True)
class ToolRequest:
    kind: ToolKind
    arguments: str


@dataclass(frozen=True, slots=True)
class ModelRequest:
    messages: tuple[object, ...]
    policy: str
    available_tools: tuple[ToolKind, ...]


@dataclass(frozen=True, slots=True)
class ModelResponse:
    message: str
    tool_request: ToolRequest | None = None


@dataclass(frozen=True, slots=True)
class SearchRequest:
    query: str


@dataclass(frozen=True, slots=True)
class SearchResult:
    content: str


@dataclass(frozen=True, slots=True)
class ReadPageRequest:
    uri: str


@dataclass(frozen=True, slots=True)
class ReadPageResult:
    content: str


SearchWeb = Tool[SearchRequest, SearchResult]("cap.search")
ReadPage = Tool[ReadPageRequest, ReadPageResult]("cap.read")
SupportModel = Model[ModelRequest, ModelResponse]("model.target.v1")


@Context
class ConversationContext:
    messages: tuple[object, ...] = ()
    model_window: tuple[object, ...] = ()
    active_input: ConversationInput | None = None
    policy: str = "grounded-support-v1"
    model_calls: int = 0
    tool_calls: int = 0


@Hook.before(target="SupportModel", scope="model")
async def PrepareModelContext(agent) -> None:
    agent.context = ConversationContext(
        messages=agent.context.messages,
        model_window=agent.context.model_window[-24:],
        active_input=agent.context.active_input,
        policy="grounded-support-v1",
        model_calls=agent.context.model_calls,
        tool_calls=agent.context.tool_calls,
    )


@Hook.after(target="SupportModel", scope="model")
async def CountModelCall(agent) -> None:
    agent.context = ConversationContext(
        messages=agent.context.messages,
        model_window=agent.context.model_window,
        active_input=agent.context.active_input,
        policy=agent.context.policy,
        model_calls=agent.context.model_calls + 1,
        tool_calls=agent.context.tool_calls,
    )


@Hook.after(target="SearchWeb", scope="capability")
async def CountWebSearch(agent) -> None:
    agent.context = ConversationContext(
        messages=agent.context.messages,
        model_window=agent.context.model_window,
        active_input=agent.context.active_input,
        policy=agent.context.policy,
        model_calls=agent.context.model_calls,
        tool_calls=agent.context.tool_calls + 1,
    )


@Hook.after(target="ReadPage", scope="capability")
async def CountPageRead(agent) -> None:
    agent.context = ConversationContext(
        messages=agent.context.messages,
        model_window=agent.context.model_window,
        active_input=agent.context.active_input,
        policy=agent.context.policy,
        model_calls=agent.context.model_calls,
        tool_calls=agent.context.tool_calls + 1,
    )


@Agent(
    input=ConversationInput,
    output=ConversationOutput,
    context=ConversationContext,
)
async def ConversationalExample(agent, incoming):
    while True:
        if agent.context.active_input is None:
            agent.context = ConversationContext(
                messages=(*agent.context.messages, incoming),
                model_window=(*agent.context.model_window, incoming),
                active_input=incoming,
                policy=agent.context.policy,
                model_calls=agent.context.model_calls,
                tool_calls=agent.context.tool_calls,
            )

        response = await SupportModel(
            ModelRequest(
                messages=agent.context.model_window,
                policy=agent.context.policy,
                available_tools=("search_web", "read_page"),
            )
        )

        if response.tool_request is not None:
            if response.tool_request.kind == "search_web":
                tool_result = await SearchWeb(
                    SearchRequest(query=response.tool_request.arguments)
                )
            else:
                tool_result = await ReadPage(
                    ReadPageRequest(uri=response.tool_request.arguments)
                )
            agent.context = ConversationContext(
                messages=(*agent.context.messages, response, tool_result),
                model_window=(*agent.context.model_window, response, tool_result),
                active_input=agent.context.active_input,
                policy=agent.context.policy,
                model_calls=agent.context.model_calls,
                tool_calls=agent.context.tool_calls,
            )
        else:
            agent.context = ConversationContext(
                messages=(*agent.context.messages, response),
                model_window=(*agent.context.model_window, response),
                active_input=None,
                policy=agent.context.policy,
                model_calls=agent.context.model_calls,
                tool_calls=agent.context.tool_calls,
            )
            incoming = await agent.yield_(ConversationOutput(message=response.message))


def main() -> None:
    """Print the example graph, canonical AIR, or compiler diagnostics."""
    if "--air" in sys.argv[1:]:
        print(ConversationalExample.canonical_air())
    elif "--diagnostics" in sys.argv[1:]:
        print(json.dumps(ConversationalExample.diagnostics()))
    else:
        print(
            json.dumps(
                ConversationalExample.frontend_graph(),
                sort_keys=True,
                separators=(",", ":"),
            )
        )


if __name__ == "__main__":
    main()
