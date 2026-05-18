#!/usr/bin/env python3
"""HAL adapter — Hosted-API-Like shim for external agent benchmarks.

Fronts an APXM-registered backend behind an OpenAI-chat-completions
endpoint so external runners (τ²-bench, GAIA, AppWorld, SWE-bench
Verified Lite) can dispatch through APXM without modification. Plan 05
IMPL surface; see `tools/hal_adapter/README.md` and
`.apxm/docs/plans/05-agentic-accuracy.md`.

Pure stdlib (http.server) — no FastAPI or httpx dep. The shim forwards
to the actual vLLM endpoint via urllib, then post-processes the
response to add the APXM-specific paired-arm switch behavior:

- `--backend apxm-on` (default): pass-through with all APXM hints
  intact. Acts as a transparent proxy to the registered vllm-fork
  endpoint.
- `--backend flat-http`: strip the `extra_body.vllm_xargs.apxm` block
  from the request body before forwarding, equivalent to the
  `--no-apxm-hints` arm of `concurrent_matrix.py`. Lets external
  benchmark runners express the paired A/B without modification.

Logging: every launch writes a manifest line at
`.apxm/evaluation/hal/launches/<TS>.json` per Plan 05 manifest contract.
Without a manifest, claim files MUST NOT cite the shim's output.

Usage:
    python3 tools/hal_adapter/server.py --port 18080 \\
      --upstream http://localhost:<vllm-port> \\
      --backend apxm-on \\
      --model gpt-oss-120b
"""
from __future__ import annotations

import argparse
import json
import os
import sys
import time
import uuid
from datetime import datetime, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any
from urllib import error as urllib_error
from urllib import request as urllib_request

REPO_ROOT = Path(__file__).resolve().parents[2]

# Shim-server launch identity surfaced in every manifest line so claim
# files can cite the exact shim configuration that produced an eval.
HAL_ADAPTER_SHA = "hal-adapter-v0-2026-05-18"

# Backend mode constants — matches the contract in the README and the
# `--no-apxm-hints` arm semantics from concurrent_matrix.py.
BACKEND_APXM_ON = "apxm-on"
BACKEND_FLAT_HTTP = "flat-http"

# OpenAI-compat path the shim accepts. Other paths return 404.
PATH_CHAT_COMPLETIONS = "/v1/chat/completions"
PATH_HEALTH = "/health"

# Manifest sink — appended to per-launch.
MANIFEST_DIR = REPO_ROOT / ".apxm" / "evaluation" / "hal" / "launches"


def _now_iso() -> str:
    return datetime.now(timezone.utc).isoformat()


def _strip_apxm_block(body: dict) -> dict:
    """Remove the extra_body.vllm_xargs.apxm block in-place. Returns the
    same dict for chaining. Used by the flat-http arm to produce a
    paired-control request shape that hits the same vLLM binary
    without the dispatch hints."""
    extra = body.get("extra_body")
    if isinstance(extra, dict):
        xargs = extra.get("vllm_xargs")
        if isinstance(xargs, dict) and "apxm" in xargs:
            del xargs["apxm"]
            if not xargs:
                del extra["vllm_xargs"]
        if not extra:
            del body["extra_body"]
    xargs = body.get("vllm_xargs")
    if isinstance(xargs, dict) and "apxm" in xargs:
        del xargs["apxm"]
        if not xargs:
            del body["vllm_xargs"]
    return body


class _Handler(BaseHTTPRequestHandler):
    # Set by main() before serve_forever.
    upstream: str = ""
    backend_mode: str = BACKEND_APXM_ON
    model: str = ""
    manifest_path: Path | None = None

    def do_GET(self):  # noqa: N802 — stdlib API
        if self.path == PATH_HEALTH:
            self._respond_json(200, {"status": "ok", "backend": self.backend_mode})
            return
        self.send_error(404, f"Path not handled: {self.path}")

    def do_POST(self):  # noqa: N802
        if self.path != PATH_CHAT_COMPLETIONS:
            self.send_error(404, f"Path not handled: {self.path}")
            return
        length = int(self.headers.get("Content-Length", "0"))
        raw_body = self.rfile.read(length) if length else b""
        try:
            body = json.loads(raw_body) if raw_body else {}
        except json.JSONDecodeError as exc:
            self.send_error(400, f"Invalid JSON: {exc}")
            return

        if self.backend_mode == BACKEND_FLAT_HTTP:
            _strip_apxm_block(body)
        if self.model and not body.get("model"):
            body["model"] = self.model

        forwarded_body = json.dumps(body).encode("utf-8")
        url = f"{self.upstream.rstrip('/')}{PATH_CHAT_COMPLETIONS}"
        req = urllib_request.Request(
            url,
            data=forwarded_body,
            headers={"Content-Type": "application/json"},
            method="POST",
        )
        start = time.perf_counter()
        try:
            with urllib_request.urlopen(req, timeout=300) as resp:
                upstream_body = resp.read()
                upstream_status = resp.status
                upstream_headers = dict(resp.headers)
        except urllib_error.HTTPError as exc:
            upstream_body = exc.read() if exc.fp else b""
            upstream_status = exc.code
            upstream_headers = dict(exc.headers or {})
        except urllib_error.URLError as exc:
            self.send_error(502, f"Upstream unreachable: {exc.reason}")
            return
        latency_ms = (time.perf_counter() - start) * 1000.0

        self.send_response(upstream_status)
        self.send_header("Content-Type", upstream_headers.get(
            "Content-Type", "application/json"
        ))
        # Pass through APXM fork's x-apxm-fields-honored header so the
        # external benchmark runner can record per-request honor evidence
        # alongside its own per-task results.
        fields_honored = upstream_headers.get("x-apxm-fields-honored")
        if fields_honored:
            self.send_header("x-apxm-fields-honored", fields_honored)
        self.send_header("x-hal-adapter-backend", self.backend_mode)
        self.send_header("x-hal-adapter-latency-ms", f"{latency_ms:.1f}")
        self.send_header("Content-Length", str(len(upstream_body)))
        self.end_headers()
        self.wfile.write(upstream_body)

    def _respond_json(self, code: int, payload: dict[str, Any]):
        data = json.dumps(payload).encode("utf-8")
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def log_message(self, format, *args):  # noqa: A002, N802 — stdlib API
        # Quieter logs — write to stderr without per-request spam.
        sys.stderr.write(f"[hal] {self.address_string()} {format % args}\n")


def _write_manifest(args: argparse.Namespace) -> Path:
    """Per-launch manifest line. Plan 05 §IMPL — claim files MUST cite."""
    MANIFEST_DIR.mkdir(parents=True, exist_ok=True)
    ts = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    path = MANIFEST_DIR / f"{ts}-{uuid.uuid4().hex[:8]}.json"
    manifest = {
        "started_at": _now_iso(),
        "hal_adapter_sha": HAL_ADAPTER_SHA,
        "port": args.port,
        "upstream": args.upstream,
        "backend": args.backend,
        "model": args.model,
        "graph_context": args.graph_context,
        "pid": os.getpid(),
    }
    path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    return path


def _parse_args() -> argparse.Namespace:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--port", type=int, default=18080)
    p.add_argument(
        "--upstream",
        required=True,
        help=(
            "Base URL of the registered vLLM endpoint "
            "(e.g. http://localhost:<port-from-zoo-manifest>)."
        ),
    )
    p.add_argument(
        "--backend",
        choices=[BACKEND_APXM_ON, BACKEND_FLAT_HTTP],
        default=BACKEND_APXM_ON,
        help=(
            f"{BACKEND_APXM_ON}: pass APXM hints through. {BACKEND_FLAT_HTTP}: "
            "strip vllm_xargs.apxm so the upstream sees a vanilla request. "
            "Use the second for paired-arm A/B controls."
        ),
    )
    p.add_argument(
        "--model",
        default="",
        help=(
            "Bare served-model name (e.g. gpt-oss-120b). When set, the shim "
            "injects this as the request body's `model` field when missing. "
            "Bare name per the apxm-config-model-id-must-match-served rule."
        ),
    )
    p.add_argument(
        "--graph-context",
        default="",
        help=(
            "Optional graph-id stamped on the request so the benchmark's "
            "per-task multi-call sequence becomes one APXM graph for "
            "pin / cohort scheduling. Empty disables grouping."
        ),
    )
    return p.parse_args()


def main() -> int:
    args = _parse_args()
    manifest_path = _write_manifest(args)
    print(f"[hal] manifest: {manifest_path}", file=sys.stderr)

    _Handler.upstream = args.upstream
    _Handler.backend_mode = args.backend
    _Handler.model = args.model
    _Handler.manifest_path = manifest_path

    server = ThreadingHTTPServer(("127.0.0.1", args.port), _Handler)
    print(
        f"[hal] listening on http://127.0.0.1:{args.port} "
        f"backend={args.backend} upstream={args.upstream}",
        file=sys.stderr,
    )
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        print("[hal] interrupted; shutting down", file=sys.stderr)
        server.server_close()
    return 0


if __name__ == "__main__":
    sys.exit(main())
