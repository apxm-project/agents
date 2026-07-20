"""Canonical session agent package entry.

Authors the minimal conversational session on the canonical ``apxm_program``
frontend and prints canonical ``apxm.air.v1`` AIR JSON to stdout. This is the
exact stdout contract ``apxm compile-service-canonical`` captures and validates
as an ``AirModule`` before the Server session family drives it.
"""

from __future__ import annotations

import apxm_program
from apxm_program.example import session_agent_graph

if __name__ == "__main__":
    print(apxm_program.canonical_air_json(session_agent_graph()), end="")
