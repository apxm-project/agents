"""Interaction harness: compose a child, then yield versus await.event."""

from __future__ import annotations

from typing import TypedDict

from apxm_program import Workflow, Event, EventRef, Model


class HarnessInput(TypedDict):
    message: str
    approval: EventRef[HarnessOutput]


class HarnessOutput(TypedDict):
    message: str


ChildModel = Model[HarnessInput, HarnessOutput]("model.child")
Approval = Event[HarnessOutput]("event.harness.approval")


@Workflow(input=HarnessInput, output=HarnessOutput)
async def Child(agent, request):
    return await ChildModel(request)


@Workflow(input=HarnessInput, output=HarnessOutput)
async def Harness(agent, request):
    while True:
        child = Child.new()
        review = await child.invoke(request)
        approved = await Approval.wait(request["approval"])
        request = await agent.yield_(review)


if __name__ == "__main__":
    print(Harness.canonical_air(), end="")
