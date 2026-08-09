"""Canonical session agent package entry.

Authors a minimal session Agent on the typed authoring surface and prints
canonical ``apxm.air.v2`` AIR JSON to stdout. This is the exact stdout contract
``apxm compile-service-canonical`` captures and validates as an ``AirModule``
before the Server session family drives it.
"""

from __future__ import annotations

from apxm_program import Agent, Context, Event, Model

SessionModel = Model[object, object]("model.target.v1")
SessionInput = Event[object]("event.session.input")


@Context
class SessionContext:
    messages: tuple = ()


@Agent(input="SessionInput", output="SessionOutput", context=SessionContext)
async def SessionAgent(agent, incoming):
    while True:
        reply = await SessionModel(incoming)
        incoming = await SessionInput.wait()


if __name__ == "__main__":
    print(SessionAgent.canonical_air(), end="")
