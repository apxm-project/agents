"""Interaction harness: compose a child, then yield versus await.event."""

from __future__ import annotations

from typing import TypedDict

from apxm_program import Agent, Event, Model


class HarnessInput(TypedDict):
    message: str


class HarnessOutput(TypedDict):
    message: str


ChildModel = Model[HarnessInput, HarnessOutput]("model.child")
Approval = Event[HarnessInput]("event.harness.approval")


@Agent(input=HarnessInput, output=HarnessOutput)
async def Child(agent, request):
    return await ChildModel(request)


@Agent(input=HarnessInput, output=HarnessOutput)
async def Harness(agent, request):
    while True:
        child = Child.new()
        review = await child.invoke(request)
        approved = await Approval.wait()
        request = await agent.yield_(review)


if __name__ == "__main__":
    print(Harness.canonical_air(), end="")
