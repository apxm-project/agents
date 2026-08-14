"""The Python conversational reference over generic Agent Program APIs."""

from __future__ import annotations

import json
import sys
from typing import Literal, TypeAlias, TypedDict

from apxm_program import Agent, Context, Hook, Model, Tool


class ConversationInput(TypedDict):
    message: str


class ConversationOutput(TypedDict):
    message: str


class ConversationMessage(TypedDict):
    role: Literal["user", "assistant", "tool"]
    content: str


class SearchWebRequest(TypedDict):
    query: str


class SearchWebResult(TypedDict):
    content: str


class SearchWebToolRequest(TypedDict):
    kind: Literal["search_web"]
    arguments: SearchWebRequest


class FinalModelResponse(TypedDict):
    kind: Literal["final"]
    reply: ConversationOutput


class ToolModelResponse(TypedDict):
    kind: Literal["tool_request"]
    tool_request: SearchWebToolRequest


class InitialModelRequest(TypedDict):
    messages: tuple[ConversationMessage, ...]
    incoming: ConversationInput


class ToolResultModelRequest(InitialModelRequest):
    tool_result: SearchWebResult


ModelRequest: TypeAlias = InitialModelRequest | ToolResultModelRequest
ModelResponse: TypeAlias = FinalModelResponse | ToolModelResponse

SearchWeb = Tool[SearchWebRequest, SearchWebResult]("cap.search")
SupportModel = Model[ModelRequest, ModelResponse]("model.target")


@Context
class ConversationContext:
    messages: tuple[ConversationMessage, ...] = ()
    tool_calls: int = 0
    last_reply: str = ""


@Hook.before(target="SearchWeb", scope="capability")
async def PrepareSearchContext(agent) -> None:
    agent.context = ConversationContext(
        messages=agent.context.messages[-24:],
        tool_calls=agent.context.tool_calls,
    )


@Hook.after(target="SearchWeb", scope="capability")
async def RecordSearchContext(agent) -> None:
    agent.context = ConversationContext(
        messages=agent.context.messages,
        tool_calls=agent.context.tool_calls + 1,
    )


@Agent(
    input="ConversationInput",
    output="ConversationOutput",
    context=ConversationContext,
)
async def ConversationalExample(agent, incoming):
    while incoming["message"] != "":
        response = await SupportModel(
            {"messages": agent.context.messages, "incoming": incoming}
        )

        while response["kind"] == "tool_request":
            if response["tool_request"]["kind"] == "search_web":
                tool_result = await SearchWeb(response["tool_request"]["arguments"])
                response = await SupportModel(
                    {
                        "messages": agent.context.messages,
                        "incoming": incoming,
                        "tool_result": tool_result,
                    }
                )
            else:
                raise ValueError("undeclared tool request")

        if response["kind"] != "final":
            raise ValueError("undeclared model response")

        agent.context = ConversationContext(
            messages=agent.context.messages,
            tool_calls=agent.context.tool_calls,
            last_reply=response["reply"]["message"],
        )
        incoming = await agent.yield_(response["reply"])
    raise ValueError("missing conversation input")


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
