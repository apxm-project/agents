"""The Python conversational reference over generic Agent Program APIs."""

from __future__ import annotations

import json
import sys
from typing import Literal, TypeAlias, TypedDict

from apxm_program import Agent, Capability, Context, Hook, Model, Skill, Tool
from apxm_program.capabilities import COUNT_TOKENS, SEARCH_WEB
from apxm_program.permissions import Allow, Ask
from apxm_program.scopes import CAPABILITY


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
    over_budget: bool


class CompactionRequest(TypedDict):
    messages: tuple[ConversationMessage, ...]
    budget: CountTokensResult


class CompactionResult(TypedDict):
    messages: tuple[ConversationMessage, ...]


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
    persona: str
    context_policy: str
    messages: tuple[ConversationMessage, ...]
    incoming: ConversationInput


class ToolResultModelRequest(InitialModelRequest):
    tool_result: SearchWebResult


ModelRequest: TypeAlias = InitialModelRequest | ToolResultModelRequest
ModelResponse: TypeAlias = FinalModelResponse | ToolModelResponse

# Measuring the model-visible conversation is a Capability, not host code, so
# the measurement is an ordinary `capability.invoke` the compiler can see. The
# permission each binding carries is what the *program* asks for; a deployment
# may narrow it in `agent.toml [permissions]` and may never widen it.
CountTokens = Capability[CountTokensRequest, CountTokensResult](
    COUNT_TOKENS,
    permission=Allow("Counts tokens in the conversation this program already holds."),
)
SearchWeb = Tool[SearchWebRequest, SearchWebResult](
    SEARCH_WEB,
    permission=Ask("Sends a model-chosen query to a third-party search index."),
)
SupportModel = Model[ModelRequest, ModelResponse]("model.target")

# The second declared Model. It is called from a Hook body, so compaction is
# workflow structure the compiler sees rather than host code behind a digest.
CompactConversation = Model[CompactionRequest, CompactionResult]("model.compaction")

# The persona and the context policy are instructions, which is what a Skill
# is. Loading one is an ordinary `capability.invoke` on `read_skill`.
PersonaSkill = Skill("persona", entry="skills/persona/SKILL.md")
ContextPolicySkill = Skill("context-policy", entry="skills/context-policy/SKILL.md")


@Context
class ConversationContext:
    messages: tuple[ConversationMessage, ...] = ()
    last_reply: str = ""
    context_budget: CountTokensResult | None = None
    last_tool: str = ""


@Hook.before(target=SearchWeb, scope=CAPABILITY)
async def PrepareSearchContext(agent) -> None:
    """Measure the conversation and hand the measurement to the compactor."""
    budget = await CountTokens({"messages": agent.context.messages})
    compacted = await CompactConversation(
        {"messages": agent.context.messages, "budget": budget}
    )
    agent.context = ConversationContext(
        messages=compacted["messages"],
        last_reply=agent.context.last_reply,
        context_budget=budget,
        last_tool=agent.context.last_tool,
    )


@Hook.after(target=SearchWeb, scope=CAPABILITY)
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
    persona = await PersonaSkill.load()
    context_policy = await ContextPolicySkill.load()
    while incoming["message"] != "":
        response = await SupportModel(
            {
                "persona": persona,
                "context_policy": context_policy,
                "messages": agent.context.messages,
                "incoming": incoming,
            }
        )

        while response["kind"] == "tool_request":
            if response["tool_request"]["kind"] == "search_web":
                tool_result = await SearchWeb(response["tool_request"]["arguments"])
                response = await SupportModel(
                    {
                        "persona": persona,
                        "context_policy": context_policy,
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
