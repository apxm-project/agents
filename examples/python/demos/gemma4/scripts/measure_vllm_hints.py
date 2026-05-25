#!/usr/bin/env python3
"""Direct vLLM measurements for APXM scheduling/cache hints.

This script talks to the OpenAI-compatible vLLM server directly. It is not an
APXM benchmark; it answers whether the underlying backend honors the controls
that APXM lowers into requests.
"""

from __future__ import annotations

import argparse
import concurrent.futures
import json
import statistics
import sys
import time
import uuid
from pathlib import Path
from typing import Any
from urllib import error, request


def _find_repo_root(start: Path) -> Path:
    for candidate in (start.resolve(), *start.resolve().parents):
        if (candidate / "Cargo.toml").is_file() and (candidate / "tools").is_dir():
            return candidate
    return Path.cwd().resolve()


REPO_ROOT = _find_repo_root(Path(__file__))
APXM_PKG_DIR = REPO_ROOT / "crates" / "compiler" / "apxm-frontend" / "python"
if str(APXM_PKG_DIR) not in sys.path:
    sys.path.insert(0, str(APXM_PKG_DIR))

from apxm.contract import (  # noqa: E402
    ApiRoute,
    HttpHeader,
    HttpMethod,
    MediaType,
    VllmDefaults,
    WireKey,
    local_endpoint,
)


DEFAULTS = VllmDefaults()
DEFAULT_BASE_URL = local_endpoint(host=DEFAULTS.host, port=DEFAULTS.port)
SELECTED_COUNTERS = (
    "vllm:prefix_cache_queries_total",
    "vllm:prefix_cache_hits_total",
    "vllm:prompt_tokens_total",
    "vllm:prompt_tokens_cached_total",
    "vllm:generation_tokens_total",
    "vllm:request_success_total",
)


def _root_url(base_url: str) -> str:
    base = base_url.rstrip("/")
    suffix = f"/{ApiRoute.OPENAI_PREFIX.value}"
    if base.endswith(suffix):
        return base[: -len(suffix)]
    return base


def _json_request(
    method: str | HttpMethod,
    url: str,
    payload: dict[str, Any] | None = None,
    timeout: float = 600.0,
) -> Any:
    data = json.dumps(payload).encode("utf-8") if payload is not None else None
    req = request.Request(
        url,
        data=data,
        method=method.value if isinstance(method, HttpMethod) else method,
        headers={HttpHeader.CONTENT_TYPE.value: MediaType.JSON.value},
    )
    try:
        with request.urlopen(req, timeout=timeout) as response:
            raw = response.read().decode("utf-8")
    except error.HTTPError as exc:
        body = exc.read().decode("utf-8", errors="replace")
        raise RuntimeError(f"{method} {url} failed: HTTP {exc.code}: {body}") from exc
    if not raw:
        return None
    return json.loads(raw)


def _text_request(url: str, timeout: float = 30.0) -> str:
    with request.urlopen(url, timeout=timeout) as response:
        return response.read().decode("utf-8")


def _wait_for_server(base_url: str, timeout_s: float) -> None:
    deadline = time.monotonic() + timeout_s
    last_error: Exception | None = None
    while time.monotonic() < deadline:
        try:
            _json_request(
                HttpMethod.GET,
                f"{base_url.rstrip('/')}/{ApiRoute.MODELS.value}",
                timeout=10.0,
            )
            return
        except Exception as exc:  # noqa: BLE001 - report the final connection error.
            last_error = exc
            time.sleep(2.0)
    raise RuntimeError(f"vLLM server did not become ready: {last_error}")


def _model_id(base_url: str, explicit_model: str | None) -> str:
    if explicit_model:
        return explicit_model
    data = _json_request(
        HttpMethod.GET,
        f"{base_url.rstrip('/')}/{ApiRoute.MODELS.value}",
        timeout=30.0,
    )
    models = data.get("data") if isinstance(data, dict) else None
    if not models:
        raise RuntimeError(
            "Unable to discover model id from "
            f"/{ApiRoute.OPENAI_PREFIX.value}/{ApiRoute.MODELS.value}"
        )
    return str(models[0]["id"])


def _metrics_snapshot(metrics_url: str | None) -> dict[str, float]:
    if not metrics_url:
        return {}
    try:
        text = _text_request(metrics_url)
    except Exception:
        return {}

    counters: dict[str, float] = {}
    for line in text.splitlines():
        if line.startswith("#"):
            continue
        for name in SELECTED_COUNTERS:
            if line.startswith(name):
                value = float(line.rsplit(" ", 1)[-1])
                counters[name] = counters.get(name, 0.0) + value
    return counters


def _metrics_delta(before: dict[str, float], after: dict[str, float]) -> dict[str, float]:
    keys = sorted(set(before) | set(after))
    return {key: after.get(key, 0.0) - before.get(key, 0.0) for key in keys}


def _usage(response: dict[str, Any]) -> dict[str, int | None]:
    usage = response.get("usage") if isinstance(response, dict) else None
    if not isinstance(usage, dict):
        return {
            "prompt_tokens": None,
            "completion_tokens": None,
            "cached_tokens": None,
        }
    details = usage.get("prompt_tokens_details")
    cached_tokens = None
    if isinstance(details, dict) and isinstance(details.get("cached_tokens"), int):
        cached_tokens = details["cached_tokens"]
    return {
        "prompt_tokens": usage.get("prompt_tokens"),
        "completion_tokens": usage.get("completion_tokens"),
        "cached_tokens": cached_tokens,
    }


def _chat(
    base_url: str,
    model: str,
    prompt: str,
    *,
    max_tokens: int,
    cache_salt: str,
    priority: int | None = None,
    apxm_class: str | None = None,
    graph_id: str | None = None,
    node_id: int | None = None,
    timeout: float = 600.0,
) -> dict[str, Any]:
    payload: dict[str, Any] = {
        "model": model,
        "messages": [{"role": "user", "content": prompt}],
        "temperature": 0,
        "max_tokens": max_tokens,
        "cache_salt": cache_salt,
    }
    if priority is not None:
        payload["priority"] = priority
    if apxm_class is not None:
        payload["vllm_xargs"] = {
            "apxm": {
                "schema_version": 1,
                "graph_id": graph_id or f"direct-{uuid.uuid4()}",
                "node_id": node_id,
                "priority_class": apxm_class,
                WireKey.PIN_POLICY.value: {WireKey.PIN_MODE.value: "none"},
            }
        }

    start = time.perf_counter()
    response = _json_request(
        HttpMethod.POST,
        f"{base_url.rstrip('/')}/{ApiRoute.CHAT_COMPLETIONS.value}",
        payload,
        timeout=timeout,
    )
    return {
        "duration_ms": (time.perf_counter() - start) * 1000.0,
        "usage": _usage(response),
        "finish_reason": response.get("choices", [{}])[0].get("finish_reason"),
    }


def _long_prefix(facts: int) -> str:
    lines = [
        "You are evaluating a deterministic APXM/vLLM prefix-cache diagnostic.",
        "Return the single word OK.",
    ]
    for index in range(facts):
        lines.append(
            "prefix fact "
            f"{index:05d}: checkout retry storm, idempotency, fulfillment, "
            "support tickets, audit records, queue pressure, rollback guards."
        )
    return "\n".join(lines)


def run_prefix_case(args: argparse.Namespace, model: str) -> dict[str, Any]:
    metrics_url = args.metrics_url
    salt = f"apxm-direct-prefix-{uuid.uuid4()}"
    prompt = _long_prefix(args.prefix_facts)

    before = _metrics_snapshot(metrics_url)
    cold = _chat(
        args.base_url,
        model,
        prompt,
        max_tokens=args.prefix_max_tokens,
        cache_salt=salt,
        timeout=args.request_timeout,
    )
    mid = _metrics_snapshot(metrics_url)
    warm = _chat(
        args.base_url,
        model,
        prompt,
        max_tokens=args.prefix_max_tokens,
        cache_salt=salt,
        timeout=args.request_timeout,
    )
    after = _metrics_snapshot(metrics_url)

    return {
        "salt": salt,
        "cold": cold,
        "warm": warm,
        "speedup": cold["duration_ms"] / warm["duration_ms"]
        if warm["duration_ms"] > 0
        else None,
        "metrics_cold_delta": _metrics_delta(before, mid),
        "metrics_warm_delta": _metrics_delta(mid, after),
        "metrics_total_delta": _metrics_delta(before, after),
    }


def _pressure_prompt(lane: int, facts: int) -> str:
    lines = [
        f"Background queue-pressure lane {lane}.",
        "Produce four concise operational findings and stop.",
    ]
    for index in range(facts):
        lines.append(
            f"lane={lane} fact={index:05d}: retry timeout, provider slowness, "
            "duplicate authorization, warehouse delay, notification suppression, "
            "finance auditability, rollback metric."
        )
    return "\n".join(lines)


def _critical_prompt() -> str:
    return (
        "A production checkout incident is active. Return the safest immediate "
        "mitigation in one sentence and stop."
    )


def run_priority_scenario(
    args: argparse.Namespace,
    model: str,
    *,
    hinted: bool,
) -> dict[str, Any]:
    scenario = "hinted" if hinted else "control"
    run_id = f"apxm-direct-priority-{scenario}-{uuid.uuid4()}"
    before = _metrics_snapshot(args.metrics_url)
    with concurrent.futures.ThreadPoolExecutor(
        max_workers=args.background_requests + 1
    ) as executor:
        background_futures = []
        for lane in range(args.background_requests):
            if hinted:
                priority = 5
                apxm_class = "parallel"
            else:
                priority = 0
                apxm_class = None
            background_futures.append(
                executor.submit(
                    _chat,
                    args.base_url,
                    model,
                    _pressure_prompt(lane, args.background_facts),
                    max_tokens=args.background_max_tokens,
                    cache_salt=f"{run_id}-background-{lane}",
                    priority=priority,
                    apxm_class=apxm_class,
                    graph_id=run_id,
                    node_id=lane + 1,
                    timeout=args.request_timeout,
                )
            )

        time.sleep(args.critical_delay_s)
        critical_future = executor.submit(
            _chat,
            args.base_url,
            model,
            _critical_prompt(),
            max_tokens=args.critical_max_tokens,
            cache_salt=f"{run_id}-critical",
            priority=0,
            apxm_class="critical_path" if hinted else None,
            graph_id=run_id,
            node_id=10_000,
            timeout=args.request_timeout,
        )
        critical = critical_future.result()
        background = [future.result() for future in background_futures]
    after = _metrics_snapshot(args.metrics_url)

    background_ms = [item["duration_ms"] for item in background]
    return {
        "scenario": scenario,
        "run_id": run_id,
        "critical": critical,
        "background_count": len(background),
        "background_duration_ms_mean": statistics.fmean(background_ms)
        if background_ms
        else None,
        "background_duration_ms_max": max(background_ms) if background_ms else None,
        "background_cached_tokens_total": sum(
            item["usage"].get("cached_tokens") or 0 for item in background
        ),
        "metrics_delta": _metrics_delta(before, after),
    }


def run_priority_case(args: argparse.Namespace, model: str) -> dict[str, Any]:
    control = run_priority_scenario(args, model, hinted=False)
    hinted = run_priority_scenario(args, model, hinted=True)
    control_ms = control["critical"]["duration_ms"]
    hinted_ms = hinted["critical"]["duration_ms"]
    return {
        "control": control,
        "hinted": hinted,
        "critical_speedup": control_ms / hinted_ms if hinted_ms > 0 else None,
        "critical_reduction_ms": control_ms - hinted_ms,
        "critical_reduction_percent": ((control_ms - hinted_ms) / control_ms * 100.0)
        if control_ms > 0
        else None,
    }


def add_contract_aliases(result: dict[str, Any]) -> None:
    aliases: dict[str, Any] = {}
    prefix = result.get("prefix")
    if isinstance(prefix, dict):
        cold_usage = prefix.get("cold", {}).get("usage", {})
        warm_usage = prefix.get("warm", {}).get("usage", {})
        if isinstance(cold_usage, dict):
            aliases["prefix_cold_cached_input_tokens"] = cold_usage.get("cached_tokens")
        if isinstance(warm_usage, dict):
            aliases["prefix_warm_cached_input_tokens"] = warm_usage.get("cached_tokens")
            aliases["cached_input_tokens"] = warm_usage.get("cached_tokens")

    priority = result.get("priority")
    if isinstance(priority, dict):
        aliases["priority_control_critical_duration_ms"] = (
            priority.get("control", {}).get("critical", {}).get("duration_ms")
        )
        aliases["priority_hinted_critical_duration_ms"] = (
            priority.get("hinted", {}).get("critical", {}).get("duration_ms")
        )
        aliases["priority_critical_reduction_ms"] = priority.get("critical_reduction_ms")
        aliases["priority_critical_reduction_percent"] = priority.get(
            "critical_reduction_percent"
        )

    result["contract_aliases"] = aliases


def write_summary(path: Path, result: dict[str, Any]) -> None:
    lines = [
        "# Direct vLLM Hint Measurement",
        "",
        "This is a backend-only measurement. APXM is not in the path.",
        "",
    ]
    if "prefix" in result:
        prefix = result["prefix"]
        lines.extend(
            [
                "## Prefix Cache",
                "",
                f"- Cold latency: {prefix.get('cold', {}).get('duration_ms', 0):.1f} ms",
                f"- Warm latency: {prefix.get('warm', {}).get('duration_ms', 0):.1f} ms",
                f"- Speedup: {prefix.get('speedup', 0):.2f}x",
                f"- Cold usage: `{json.dumps(prefix.get('cold', {}).get('usage', {}), sort_keys=True)}`",
                f"- Warm usage: `{json.dumps(prefix.get('warm', {}).get('usage', {}), sort_keys=True)}`",
                "",
            ]
        )
    if "priority" in result:
        priority = result["priority"]
        lines.extend(
            [
                "## Priority Under Queue Pressure",
                "",
                f"- Control critical latency: {priority.get('control', {}).get('critical', {}).get('duration_ms', 0):.1f} ms",
                f"- Hinted critical latency: {priority.get('hinted', {}).get('critical', {}).get('duration_ms', 0):.1f} ms",
                f"- Critical speedup: {priority.get('critical_speedup', 0):.2f}x",
                f"- Reduction: {priority.get('critical_reduction_ms', 0):.1f} ms",
                f"- Reduction percent: {priority.get('critical_reduction_percent', 0):.1f}%",
            ]
        )
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base-url", default=DEFAULT_BASE_URL)
    parser.add_argument("--metrics-url")
    parser.add_argument("--model")
    parser.add_argument("--wait", type=float, default=0.0)
    parser.add_argument("--request-timeout", type=float, default=900.0)
    parser.add_argument("--prefix-facts", type=int, default=1200)
    parser.add_argument("--prefix-max-tokens", type=int, default=1)
    parser.add_argument("--background-requests", type=int, default=6)
    parser.add_argument("--background-facts", type=int, default=900)
    parser.add_argument("--background-max-tokens", type=int, default=96)
    parser.add_argument("--critical-max-tokens", type=int, default=16)
    parser.add_argument("--critical-delay-s", type=float, default=0.25)
    parser.add_argument("--skip-prefix", action="store_true")
    parser.add_argument("--skip-priority", action="store_true")
    parser.add_argument("--output-json", type=Path)
    parser.add_argument("--output-md", type=Path)
    args = parser.parse_args()
    if args.metrics_url is None:
        args.metrics_url = f"{_root_url(args.base_url)}/metrics"
    return args


def main() -> None:
    args = parse_args()
    if args.wait > 0:
        _wait_for_server(args.base_url, args.wait)
    model = _model_id(args.base_url, args.model)
    result: dict[str, Any] = {
        "base_url": args.base_url,
        "metrics_url": args.metrics_url,
        "model": model,
        "timestamp_unix": time.time(),
    }
    if not args.skip_prefix:
        result["prefix"] = run_prefix_case(args, model)
    if not args.skip_priority:
        result["priority"] = run_priority_case(args, model)
    add_contract_aliases(result)

    if args.output_json:
        args.output_json.parent.mkdir(parents=True, exist_ok=True)
        args.output_json.write_text(json.dumps(result, indent=2, sort_keys=True), encoding="utf-8")
    if args.output_md:
        args.output_md.parent.mkdir(parents=True, exist_ok=True)
        write_summary(args.output_md, result)
    print(json.dumps(result, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
