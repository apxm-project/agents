#!/usr/bin/env python3
"""Pin-state verification for multi-node Ray vLLM deployments.

The APXM router lives in the API-server process (rank 0 of pipeline-parallel
groups). Graph registration and pin accounting happen on rank 0; worker
ranks (>0) host Ray workers that execute forward passes. The cross-shard
question this script answers: when a graph is registered and a pinned
request runs through the multi-node pipeline, does the rank-0 scheduler
correctly report pinned_blocks > 0 — i.e. the pin map survived the
distributed admission path that the patch stack added?

The script:

  1. Registers a graph at the rank-0 API endpoint.
  2. Drives a chat-completion request whose `vllm_xargs.apxm` carries
     a critical-path hint (so the pin policy actually engages).
  3. Queries `GET /v1/apxm/graphs/{id}` — pinned_blocks must be > 0.
  4. (Diagnostic) greps rank-0's Slurm stdout for the fork's
     `Released APXM pin ... graph_id=<id>` log line, which the scheduler
     emits at debug level when blocks are reclaimed. Helps explain
     unexpected counts without being load-bearing.

Run this against a freshly-started multi-node service (e.g. Kimi-K2 PP=3)
before adding the model to `deploy/vllm/zoo.toml`. Failure means pin
accounting regressed end-to-end across the distributed path.

Usage:
    APXM_ENDPOINT=http://kimi-head:8940 \\
    APXM_VLLM_SERVICE_NAME=vllm-kimi \\
    python3 tools/scripts/verify_cross_shard_pins.py
"""

from __future__ import annotations

import argparse
import json
import os
import re
import sys
import time
import urllib.error
import urllib.request
import uuid
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from apxm_vllm_contract import (  # noqa: E402
    ApiRoute,
    EnvVar,
    build_layout,
    env_name,
)


LAYOUT = build_layout(__file__)


def _http_json(method: str, url: str, body: dict | None = None, timeout: float = 30.0) -> dict:
    data = json.dumps(body).encode("utf-8") if body is not None else None
    req = urllib.request.Request(
        url,
        method=method,
        data=data,
        headers={"content-type": "application/json"},
    )
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        return json.loads(resp.read().decode("utf-8"))


def _resolve_service_state(service_name: str) -> dict:
    state_path = LAYOUT.service_dir / f"{service_name}.json"
    if not state_path.is_file():
        raise SystemExit(f"service state not found: {state_path}")
    with state_path.open("r", encoding="utf-8") as handle:
        return json.load(handle)


def _per_rank_log_glob(service_name: str, job_id: str) -> list[Path]:
    pattern = f"slurm-apxm-vllm-service-{service_name}-{job_id}.node*.out"
    return sorted(LAYOUT.log_dir.glob(pattern))


def _release_log_lines(log: Path, graph_id: str) -> list[str]:
    """Return every "Released APXM pin ... graph_id=<id>" line in `log`."""
    if not log.is_file():
        return []
    pattern = re.compile(rf"Released APXM pin\b.*?graph_id={re.escape(graph_id)}\b")
    return [line for line in log.read_text(errors="replace").splitlines() if pattern.search(line)]


def _make_graph(node_count: int) -> dict:
    graph_id = f"verify-pin-{uuid.uuid4().hex[:8]}"
    execution_id = f"exec-verify-{uuid.uuid4().hex[:8]}"
    return {
        "graph_id": graph_id,
        "execution_id": execution_id,
        "nodes": [
            {
                "node_id": i,
                "node_name": f"verify-node-{i}",
                "downstream_nodes": [j for j in range(node_count) if j != i],
                "priority_class": "critical_path",
                "pin_policy": "per_request",
            }
            for i in range(node_count)
        ],
    }


def _drive_request(endpoint: str, model: str, graph: dict) -> None:
    chat_url = f"{endpoint.rstrip('/')}/v1/{ApiRoute.CHAT_COMPLETIONS.value}"
    payload = {
        "model": model,
        "messages": [
            {"role": "user", "content": "verify pin accounting across distributed admission"}
        ],
        "max_tokens": 16,
        "extra_body": {
            "vllm_xargs": {
                "apxm": {
                    "schema_version": 1,
                    "graph_id": graph["graph_id"],
                    "execution_id": graph["execution_id"],
                    "node_id": graph["nodes"][0]["node_id"],
                    "node_name": graph["nodes"][0]["node_name"],
                    "downstream_nodes": graph["nodes"][0]["downstream_nodes"],
                    "priority_class": "critical_path",
                    "pin_policy": "per_request",
                }
            }
        },
    }
    _http_json("POST", chat_url, payload, timeout=120.0)


def verify(*, endpoint: str, service_name: str, node_count: int) -> int:
    state = _resolve_service_state(service_name)
    job_id = state.get("job_id")
    if not job_id:
        raise SystemExit(f"service {service_name!r} state has no job_id")
    model = state.get("served_model_name") or state.get("model")
    if not model:
        raise SystemExit(f"service {service_name!r} state has no served-model name")
    nodes = int(state.get("nodes") or 0)
    if nodes < 2:
        raise SystemExit(
            f"service {service_name!r} is single-node (nodes={nodes}); "
            f"pin verification only applies to multi-node deployments."
        )

    graph = _make_graph(node_count)
    print(
        f"[verify] graph_id={graph['graph_id']} synthetic_nodes={node_count} "
        f"service={service_name} pp_nodes={nodes}"
    )

    register_url = (
        f"{endpoint.rstrip('/')}/v1/{ApiRoute.APXM_GRAPHS.value}/"
        f"{ApiRoute.APXM_REGISTER.value}"
    )
    _http_json(
        "POST",
        register_url,
        {
            "graph_id": graph["graph_id"],
            "execution_id": graph["execution_id"],
            "nodes": graph["nodes"],
        },
    )

    _drive_request(endpoint, model, graph)
    # Give the scheduler a chance to commit pin state to its tracker.
    time.sleep(2.0)

    status_url = f"{endpoint.rstrip('/')}/v1/{ApiRoute.APXM_GRAPHS.value}/{graph['graph_id']}"
    pin_status = _http_json("GET", status_url)
    pinned_blocks = int(pin_status.get("pinned_blocks") or 0)
    pinned_handles = int(pin_status.get("pinned_handles") or 0)
    print(
        f"[verify] post-request pin status: handles={pinned_handles} "
        f"blocks={pinned_blocks}"
    )

    release_url = status_url
    release_resp = _http_json("DELETE", release_url)
    released = int(release_resp.get("released_blocks") or 0)
    print(f"[verify] release returned released_blocks={released}")

    # Diagnostic: surface the rank-0 release log to make pin lifecycle visible.
    logs = _per_rank_log_glob(service_name, str(job_id))
    if logs:
        rank_zero = next(
            (log for log in logs if ".node0" in log.name),
            logs[0],
        )
        release_lines = _release_log_lines(rank_zero, graph["graph_id"])
        if release_lines:
            print(
                f"[verify] rank-0 log records {len(release_lines)} "
                f"release event(s) for this graph:"
            )
            for line in release_lines[:5]:
                print(f"  {line}")
        else:
            print(
                f"[verify] no `Released APXM pin ... graph_id={graph['graph_id']}` "
                f"lines in rank-0 log. The scheduler only logs at DEBUG level — "
                f"either the fork's log level is INFO or pin release went through "
                f"the bulk-graph-release path."
            )
    else:
        print(
            f"[verify] no per-rank logs at "
            f"{LAYOUT.log_dir}/slurm-apxm-vllm-service-{service_name}-{job_id}.node*.out — "
            f"controller-side diagnostic skipped."
        )

    if pinned_blocks <= 0:
        print(
            f"\nFAIL: rank-0 reported pinned_blocks={pinned_blocks} for graph "
            f"{graph['graph_id']!r}. Pin accounting is broken end-to-end across "
            f"the distributed admission path; do not commit this model to the "
            f"production manifest."
        )
        return 1
    print(
        f"\nOK: rank-0 scheduler observed pin state (pinned_blocks={pinned_blocks}, "
        f"pinned_handles={pinned_handles}) for graph {graph['graph_id']!r}."
    )
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--endpoint",
        default=os.environ.get("APXM_ENDPOINT", ""),
        help="vLLM service endpoint (e.g. http://kimi-head:8940). "
        "Falls back to the APXM_ENDPOINT env var.",
    )
    parser.add_argument(
        "--service-name",
        default=os.environ.get(env_name(EnvVar.APXM_VLLM_SERVICE_NAME), ""),
        help="Zoo service name (used to locate state + per-rank log files). "
        "Falls back to APXM_VLLM_SERVICE_NAME.",
    )
    parser.add_argument(
        "--node-count",
        type=int,
        default=8,
        help="Number of synthetic graph nodes to register (default 8).",
    )
    args = parser.parse_args()

    if not args.endpoint:
        raise SystemExit("--endpoint or APXM_ENDPOINT is required")
    if not args.service_name:
        raise SystemExit("--service-name or APXM_VLLM_SERVICE_NAME is required")

    return verify(
        endpoint=args.endpoint,
        service_name=args.service_name,
        node_count=args.node_count,
    )


if __name__ == "__main__":
    sys.exit(main())
