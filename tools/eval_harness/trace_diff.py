"""Compare two CallTrace JSON files emitted by the runtime.

Used during tier-1 investigations to determine whether two execution
traces are semantically equivalent under the canonicalization rule
(sort each event's parent_deps; sort events by
(parent_deps, prompt, model, params, op, node_id)).

Exit codes:
    0  traces are equivalent           ("equivalent")
    1  traces diverge                  ("divergent_at_node:<node_id>")
    2  bad arguments / IO failure
"""
from __future__ import annotations

import json
import sys
from pathlib import Path


def canonicalize(trace: dict) -> list[tuple]:
    """Sort events by (sorted parent_deps, prompt, node_id) so independent
    reorderings collapse but dependency violations remain detectable."""
    events = []
    for e in trace.get("events", []):
        deps = tuple(sorted(e.get("parent_deps", [])))
        events.append((
            deps,
            e.get("prompt", ""),
            e.get("model", ""),
            e.get("params", ""),
            e.get("op", ""),
            e.get("node_id"),
        ))
    events.sort()
    return events


def main() -> int:
    if len(sys.argv) != 3:
        print(
            "usage: python -m eval_harness.trace_diff <a.json> <b.json>",
            file=sys.stderr,
        )
        return 2
    a = json.loads(Path(sys.argv[1]).read_text())
    b = json.loads(Path(sys.argv[2]).read_text())
    ca, cb = canonicalize(a), canonicalize(b)
    if ca == cb:
        print("equivalent")
        return 0
    for ea, eb in zip(ca, cb):
        if ea != eb:
            print(f"divergent_at_node:{ea[-1]}")
            return 1
    extra = ca[len(cb):] or cb[len(ca):]
    print(f"divergent_at_node:{extra[0][-1]}")
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
