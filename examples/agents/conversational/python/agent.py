"""A conversational Agent authored on the installed typed frontend.

A conversational Agent is an ordinary Agent: a typed input, an authored loop that
calls a Model and an optional Tool, an explicit Context replacement, and a reply
yielded before the next input. No conversation-specific runtime, turn type, or
hidden loop is involved.
"""

from __future__ import annotations

import json
import sys

from apxm_program import Agent, Context, Event, Model, Tool

SearchWeb = Tool[object, object]("cap.search")
SupportModel = Model[object, object]("model.default")
NextInput = Event[object]


@Context
class ConversationContext:
    messages: tuple = ()


@Agent(
    input="ConversationInput",
    output="ConversationOutput",
    context=ConversationContext,
)
async def ConversationalExample(agent, incoming):
    while True:
        research = await SearchWeb(incoming)
        response = await SupportModel(incoming)
        agent.context = ConversationContext()
        incoming = await NextInput.wait()


def main() -> None:
    """Print the example graph, or canonical AIR with ``--air``."""
    if "--air" in sys.argv[1:]:
        print(ConversationalExample.canonical_air())
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
