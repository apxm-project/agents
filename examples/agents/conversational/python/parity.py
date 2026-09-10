"""Paired Python source covering the complete canonical frontend seam."""

from __future__ import annotations

import json
import sys
from typing import TypedDict

from apxm_program import Workflow, Capability, Context, Event, EventRef, Hook, Model, TaskGroup, Tool
from apxm_program.capabilities import COUNT_TOKENS, SEARCH_WEB
from apxm_program.scopes import MODEL


class Input(TypedDict):
    event: EventRef[Output]


class Output(TypedDict):
    message: str


ParityModel = Model[Input, Output]("parity.model")
ParityTool = Tool[object, Output](SEARCH_WEB)
ParityCapability = Capability[object, Output](COUNT_TOKENS)
ParityEvent = Event[Output]("parity.event")


@Context
class ParityContext:
    iterations: int = 0


@Workflow(input=Input, output=Output, context=ParityContext)
async def ParityChild(agent, request):
    return await ParityModel(request)


@Hook.before(target="ParityModel", scope=MODEL)
async def RecordParityModelStart(agent) -> None:
    return None


@Workflow(input=Input, output=Output, context=ParityContext)
async def ParityCorpus(agent, request):
    while True:
        try:
            if request is not None:
                async with TaskGroup():
                    tool_result = await ParityTool(None)
            else:
                tool_result = await ParityCapability(None)
        except Exception:
            tool_result = await ParityCapability(None)
        child = ParityChild.new(context=ParityContext(iterations=0))
        child_result = await child.invoke(request)
        event_result = await ParityEvent.wait(request["event"])
        response = await ParityModel(request)
        agent.context = ParityContext(iterations=agent.context.iterations)
        request = await agent.yield_(response)


def main() -> None:
    """Print the paired graph, canonical AIR, or compiler diagnostics."""
    if "--air" in sys.argv[1:]:
        print(ParityCorpus.canonical_air())
    elif "--diagnostics" in sys.argv[1:]:
        print(json.dumps(ParityCorpus.diagnostics()))
    else:
        print(
            json.dumps(
                ParityCorpus.frontend_graph(),
                sort_keys=True,
                separators=(",", ":"),
            )
        )


if __name__ == "__main__":
    main()
