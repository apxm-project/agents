#!/usr/bin/env python3
"""Spawn a child graph as a separate execution with its own session root."""

from pathlib import Path

from apxm import GraphRecorder, NodePolicy, WorkflowTargetKind, compile

CHILD_GRAPH = Path("tests/quality_fixtures/qa_factual/graph.air")


@compile(default_policy=NodePolicy(timeout_ms=5_000))
def parent(g: GraphRecorder):
    child = g.workflow_spawn(
        name="child_review",
        target_kind=WorkflowTargetKind.GRAPH_PATH,
        target=CHILD_GRAPH,
        session_root=Path(".apxm/child-sessions"),
        node_policy=NodePolicy(timeout_ms=2_000),
    )
    g.done(child)


if __name__ == "__main__":
    print(parent._graph.to_json())
