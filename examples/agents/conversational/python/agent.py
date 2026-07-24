"""A conversational Agent authored on the installed typed frontend.

A conversational Agent is an ordinary Agent: a typed input, an authored loop that
calls a Model and an optional Tool, an explicit Context replacement, and a reply
yielded before the next input. No conversation-specific runtime, turn type, or
hidden loop is involved.
"""

from __future__ import annotations

import json
import sys

from apxm_program import Agent, Context, Model, Tool

SearchWeb = Tool[object, object]("cap.search")
SupportModel = Model[object, object]("model.target.v1")


@Context
class ResearchContext:
    requests: int = 0


@Context
class ConversationContext:
    messages: tuple = ()


@Agent(
    input="ConversationInput",
    output="ResearchOutput",
    context=ResearchContext,
)
async def ResearchSpecialist(agent, incoming):
    research = await SearchWeb(incoming)
    agent.context = ResearchContext(requests=agent.context.requests + 1)
    return research


@Agent(
    input="ConversationInput",
    output="ConversationOutput",
    context=ConversationContext,
)
async def ConversationalExample(agent, incoming):
    while True:
        specialist = ResearchSpecialist.new(context=ResearchContext())
        research = await specialist.invoke(incoming)
        response = await SupportModel({"incoming": incoming, "research": research})
        agent.context = ConversationContext(
            messages=(*agent.context.messages, incoming, response)
        )
        incoming = await agent.yield_(response)


def main() -> None:
    """Print the example graph, or canonical AIR with ``--air``."""
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
