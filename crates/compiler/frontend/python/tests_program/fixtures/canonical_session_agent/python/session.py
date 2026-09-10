"""Canonical session agent package entry.

Authors a minimal session Workflow on the typed authoring surface and prints
canonical ``apxm.air`` AIR JSON to stdout. This is the exact stdout contract
``apxm compile-service-canonical`` captures and validates as an ``AirModule``
before the Server session family drives it.
"""

from __future__ import annotations

from typing import TypedDict
from apxm_program import Workflow, Context, Event, EventRef, Model

class SessionRequest(TypedDict):
    """The typed message one session turn accepts."""
    message: str
    event: EventRef[SessionRequest]


class SessionReply:
    """The typed message one session turn produces."""


SessionModel = Model[SessionRequest, SessionReply]("model.target")
SessionInput = Event[SessionRequest]("event.session.input")


@Context
class SessionContext:
    messages: tuple = ()


@Workflow(input=SessionRequest, output=SessionReply, context=SessionContext)
async def SessionAgent(agent, incoming):
    while True:
        reply = await SessionModel(incoming)
        incoming = await SessionInput.wait(incoming["event"])


if __name__ == "__main__":
    print(SessionAgent.canonical_air(), end="")
