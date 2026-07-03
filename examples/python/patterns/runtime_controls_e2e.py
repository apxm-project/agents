#!/usr/bin/env python3
"""End-to-end example for local sessions, hooks, middleware, and workflow spawn.

Run from the repo root:

  python3 examples/python/patterns/runtime_controls_e2e.py
"""

from __future__ import annotations

import json
import os
import shutil
from pathlib import Path

from apxm import (
    ExecutionOptions,
    GraphRecorder,
    HookConfig,
    HookEvent,
    LoopGuardMiddlewareConfig,
    NodePolicy,
    TimeoutMiddlewareConfig,
    ToolGroup,
    WorkflowTargetKind,
    compile,
    emit_air_if_requested,
    find_repo_root,
    local_apxm_path,
)
from apxm.constants import ENV_APXM_MOCK_BACKEND, ENV_FLAG_ENABLED


REPO_ROOT = find_repo_root()
DEFAULT_PARENT_SESSIONS_ROOT = local_apxm_path("sessions")
CHILD_SESSIONS_ROOT = local_apxm_path("e2e-child-sessions")
HOOK_LOG = local_apxm_path("e2e-hook.log")
CHILD_GRAPH = Path(__file__).with_name("runtime_controls_child.air").resolve()
HOOK_FIELD_SEPARATOR = "|"


@compile(default_policy=NodePolicy(capability_groups=[ToolGroup.WEB], token_budget=128, timeout_ms=5_000))
def e2e_flow(g: GraphRecorder):
    summary = g.ask(
        name="summary",
        prompt="Summarize the phrase 'middleware hooks sessions' in one sentence.",
    )
    child = g.workflow_spawn(
        name="child_graph",
        target_kind=WorkflowTargetKind.AIR_PATH,
        target=CHILD_GRAPH,
        session_root=CHILD_SESSIONS_ROOT,
        node_policy=NodePolicy(
            timeout_ms=2_000,
            capability_groups=[ToolGroup.FILE_READ],
            token_budget=64,
        ),
    )
    final = g.ask(
        name="final",
        prompt="Combine this parent summary {summary} with this child result {child}.",
        policy=NodePolicy(capability_groups=[ToolGroup.WEB], token_budget=96),
    )
    g.done(final)


def _clean_artifacts() -> None:
    CHILD_SESSIONS_ROOT.parent.mkdir(parents=True, exist_ok=True)
    shutil.rmtree(CHILD_SESSIONS_ROOT, ignore_errors=True)
    HOOK_LOG.unlink(missing_ok=True)


def _graph_policy_snapshot() -> dict[str, object]:
    graph_dict = e2e_flow._graph.to_dict()
    nodes = {node["name"]: node for node in graph_dict["nodes"]}
    return {
        "summary": nodes["summary"]["attributes"],
        "child_graph": nodes["child_graph"]["attributes"],
        "final": nodes["final"]["attributes"],
    }


def _child_session_dirs() -> list[Path]:
    if not CHILD_SESSIONS_ROOT.exists():
        return []
    return sorted(path for path in CHILD_SESSIONS_ROOT.iterdir() if path.is_dir())


def _read_hook_events() -> list[dict[str, str]]:
    if not HOOK_LOG.exists():
        return []

    events: list[dict[str, str]] = []
    for line in HOOK_LOG.read_text(encoding="utf-8").splitlines():
        parts = line.split(HOOK_FIELD_SEPARATOR)
        if len(parts) != 3:
            continue
        events.append({"node_id": parts[0], "op_type": parts[1], "success": parts[2]})
    return events


def main() -> None:
    os.environ.setdefault(ENV_APXM_MOCK_BACKEND, ENV_FLAG_ENABLED)
    os.chdir(REPO_ROOT)
    _clean_artifacts()

    options = ExecutionOptions(
        hooks=[
            HookConfig(
                event=HookEvent.NODE_COMPLETE,
                command=(
                    f"printf '%s{HOOK_FIELD_SEPARATOR}%s{HOOK_FIELD_SEPARATOR}%s\\n' "
                    f"{{{{node_id}}}} {{{{op_type}}}} {{{{success}}}} >> {HOOK_LOG}"
                ),
            )
        ],
        middlewares=[
            TimeoutMiddlewareConfig(default_timeout_ms=5_000),
            LoopGuardMiddlewareConfig(max_repeats=2),
        ],
    )
    result = e2e_flow.run_sync(execution=options)

    parent_session_dir = Path(result.session_dir or "")
    child_sessions = _child_session_dirs()
    hook_events = _read_hook_events()

    if result.execution_id is None:
        raise RuntimeError("execution_id was not returned")
    if not parent_session_dir.is_dir():
        raise RuntimeError(f"parent session dir missing: {parent_session_dir}")
    if DEFAULT_PARENT_SESSIONS_ROOT not in parent_session_dir.parents:
        raise RuntimeError(
            f"parent session dir did not use local default root: {parent_session_dir}"
        )
    if len(child_sessions) != 1:
        raise RuntimeError(
            f"expected exactly one child session dir under {CHILD_SESSIONS_ROOT}, got {child_sessions}"
        )
    if CHILD_SESSIONS_ROOT not in child_sessions[0].parents:
        raise RuntimeError(f"child session dir did not use explicit root: {child_sessions[0]}")

    parent_node_count = len(e2e_flow._graph.nodes)
    if len(hook_events) < parent_node_count:
        raise RuntimeError(
            f"expected at least {parent_node_count} hook events, got {len(hook_events)}"
        )
    for event in hook_events:
        if not event["node_id"] or not event["op_type"]:
            raise RuntimeError(f"hook event contained empty fields: {hook_events}")
        if event["success"] != "true":
            raise RuntimeError(f"hook event reported non-successful completion: {hook_events}")

    summary = {
        "content": result.content,
        "execution_id": result.execution_id,
        "session_dir": str(parent_session_dir),
        "session_dir_uses_local_default_root": True,
        "session_files": sorted(path.name for path in parent_session_dir.iterdir()),
        "child_session_root": str(CHILD_SESSIONS_ROOT),
        "child_session_dirs": [str(path) for path in child_sessions],
        "child_session_files": {
            path.name: sorted(child.name for child in path.iterdir()) for path in child_sessions
        },
        "hook_log": str(HOOK_LOG),
        "hook_event_count": len(hook_events),
        "hook_events": hook_events,
        "policy_snapshot": _graph_policy_snapshot(),
    }
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    if emit_air_if_requested(e2e_flow):
        raise SystemExit(0)

    main()
