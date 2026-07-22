"""Canonical session agent package entry.

Authors the minimal conversational session on the canonical ``apxm_program``
frontend and prints canonical ``apxm.air.v1`` AIR JSON to stdout. This is the
exact stdout contract ``apxm compile-service-canonical`` captures and validates
as an ``AirModule`` before the Server session family drives it.
"""

from __future__ import annotations

import apxm_program

if __name__ == "__main__":
    program = apxm_program.AgentProgram(
        program_id="SessionAgent",
        input_type_ref="SessionInput",
        output_type_ref="SessionOutput",
        context_type_ref="SessionContext",
    )
    program.loop(
        "region.loop.session",
        lambda body: body.model_call("node.model", "model.default").await_event(
            "node.await", "event.session.input"
        ),
    )
    program.return_region("region.return")
    print(program.canonical_air_json(), end="")
