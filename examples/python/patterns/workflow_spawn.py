#!/usr/bin/env python3
"""Spawn a child AIR workflow as a separate execution with its own session root."""

from apxm import (
    GraphRecorder,
    NodePolicy,
    WorkflowTargetKind,
    compile,
    local_apxm_path,
    repo_path,
)

CHILD_AIR = repo_path("workflows", "review.air")


@compile(default_policy=NodePolicy(timeout_ms=5_000))
def parent(g: GraphRecorder):
    child = g.workflow_spawn(
        name="child_review",
        target_kind=WorkflowTargetKind.AIR_PATH,
        target=CHILD_AIR,
        session_root=local_apxm_path("child-sessions"),
        node_policy=NodePolicy(timeout_ms=2_000),
    )
    g.done(child)


if __name__ == "__main__":
    print(parent._air_text)
