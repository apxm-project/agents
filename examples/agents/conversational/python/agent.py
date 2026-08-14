"""The Python conversational reference over generic Agent Program APIs."""

from __future__ import annotations

import json
import sys
from typing import Literal, TypeAlias, TypedDict

from apxm_program import Agent, Capability, Context, Hook, Model, Tool
from apxm_program.capabilities import COUNT_TOKENS, SEARCH_WEB


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


class CountTokensRequest(TypedDict):
    messages: tuple[ConversationMessage, ...]


class CountTokensResult(TypedDict):
    total: int


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

# Measuring the model-visible conversation is a Capability, not host code, so
# the measurement is an ordinary `capability.invoke` the compiler can see.
CountTokens = Capability[CountTokensRequest, CountTokensResult](COUNT_TOKENS)
SearchWeb = Tool[SearchWebRequest, SearchWebResult](SEARCH_WEB)
SupportModel = Model[ModelRequest, ModelResponse]("model.target")


@Context
class ConversationContext:
    messages: tuple[ConversationMessage, ...] = ()
    last_reply: str = ""
    context_budget: CountTokensResult | None = None
    last_tool: str = ""


@Hook.before(target="SearchWeb", scope="capability")
async def PrepareSearchContext(agent) -> None:
    """Measure the model-visible conversation before the Tool runs."""
    budget = await CountTokens({"messages": agent.context.messages})
    agent.context = ConversationContext(
        messages=agent.context.messages,
        last_reply=agent.context.last_reply,
        context_budget=budget,
        last_tool=agent.context.last_tool,
    )


@Hook.after(target="SearchWeb", scope="capability")
async def RecordSearchContext(agent) -> None:
    """Record which Capability the conversation last dispatched."""
    agent.context = ConversationContext(
        messages=agent.context.messages,
        last_reply=agent.context.last_reply,
        context_budget=agent.context.context_budget,
        last_tool="search_web",
    )


@Agent(
    input=ConversationInput,
    output=ConversationOutput,
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
            last_reply=response["reply"]["message"],
            context_budget=agent.context.context_budget,
            last_tool=agent.context.last_tool,
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
