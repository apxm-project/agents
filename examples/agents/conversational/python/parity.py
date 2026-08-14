"""Paired Python source covering the complete canonical frontend seam."""

from __future__ import annotations

import json
import sys

from apxm_program import Agent, Capability, Context, Event, Hook, Model, TaskGroup, Tool


ParityModel = Model[object, object]("parity.model")
ParityTool = Tool[object, object]("parity.tool")
ParityCapability = Capability[object, object]("parity.capability")
ParityEvent = Event("parity.event")


@Context
class ParityContext:
    iterations: int = 0


@Agent(input="Input", output="Output", context=ParityContext)
async def ParityChild(agent, request):
    return await ParityModel(request)


@Hook.before(target="ParityModel", scope="model")
async def RecordParityModelStart(agent) -> None:
    return None


@Agent(input="Input", output="Output", context=ParityContext)
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
        event_result = await ParityEvent.wait()
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
