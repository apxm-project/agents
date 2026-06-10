#!/usr/bin/env python3
"""Run real-backend APXM MCP prompt-as-workflow dogfood tasks.

The runner intentionally stores generated evidence under .apxm so checked-in
examples and docs stay source-only.
"""

from __future__ import annotations

import argparse
import dataclasses
import datetime as dt
import enum
import json
import pathlib
import subprocess
import sys
import time
from typing import Any


class JsonRpcMethod(enum.StrEnum):
    TOOLS_CALL = "tools/call"


class McpTool(enum.StrEnum):
    PROMPT_AS_WORKFLOW = "prompt_as_workflow"


class JsonRpcField(enum.StrEnum):
    JSONRPC = "jsonrpc"
    ID = "id"
    METHOD = "method"
    PARAMS = "params"
    NAME = "name"
    ARGUMENTS = "arguments"
    RESULT = "result"
    CONTENT = "content"
    TEXT = "text"
    IS_ERROR = "isError"


class PlanArg(enum.StrEnum):
    TASK = "task"
    EXECUTE = "execute"
    TRACE_ID = "trace_id"


class PlanResult(enum.StrEnum):
    STATUS = "status"
    TRACE_ID = "trace_id"
    AIR_HASH = "air_hash"
    ARTIFACT_HASH = "artifact_hash"
    STATS = "stats"
    SUMMARY = "summary"


JSONRPC_VERSION = "2.0"
DEFAULT_SERVICE = "gptoss120b"
DEFAULT_SERVER_BIN = "target/release/apxm-mcp-server"
DEFAULT_TIMEOUT_SECONDS = 600
DEFAULT_SCENARIO = "mcp-server-prompt-as-workflow-dogfood"
UTC_SUFFIX = "Z"

DOGFOOD_TASKS: tuple[str, ...] = (
    "Create an APXM workflow to inspect README changes, identify user-facing risk, and summarize the result.",
    "Create an APXM workflow to review Cargo.toml and Cargo.lock changes for dependency or feature risk.",
    "Create an APXM workflow to audit MCP server changes for protocol compatibility and missing tests.",
    "Create an APXM workflow to inspect the prompt_as_workflow implementation and summarize validation risks.",
    "Create an APXM workflow to review query tool behavior for trace, memory, evidence, and capability lookup.",
    "Create an APXM workflow to check HTTP MCP parity with stdio MCP for all Tier-3 tools.",
    "Create an APXM workflow to inspect the Dekk MCP installer and summarize config-writer risk.",
    "Create an APXM workflow to verify stale Python/FastMCP framing has been removed from active docs.",
    "Create an APXM workflow to audit the bundled prompt-as-workflow skill resources for completeness.",
    "Create an APXM workflow to inspect schema.json and summarize contract gaps for generated workflows.",
    "Create an APXM workflow to review prompt.md for ambiguous instructions that could cause invalid JSON.",
    "Create an APXM workflow to inspect generated-workflow admission rules and summarize side-effect risks.",
    "Create an APXM workflow to review sandbox preflight handling for generated INV_TOOL nodes.",
    "Create an APXM workflow to inspect trace_id validation and summarize path traversal risk.",
    "Create an APXM workflow to review skill:// resource resolution for traversal and duplicate-id behavior.",
    "Create an APXM workflow to audit startup runtime setup for model-router initialization risk.",
    "Create an APXM workflow to inspect execution-record persistence for HTTP MCP generated workflows.",
    "Create an APXM workflow to review README documentation for new MCP tools and missing caveats.",
    "Create an APXM workflow to inspect docs/design updates for consistency with the Rust MCP direction.",
    "Create an APXM workflow to review tests/mcp.rs for gaps in prompt-as-workflow coverage.",
    "Create an APXM workflow to inspect mcp_protocol.rs and summarize scattered string risk.",
    "Create an APXM workflow to review mcp/schema.rs for accurate input schemas for new tools.",
    "Create an APXM workflow to inspect mcp/dispatch.rs for safe argument handling across query tools.",
    "Create an APXM workflow to review capability_list output and summarize model-router health visibility.",
    "Create an APXM workflow to inspect evidence_lookup allowed roots and summarize evidence exposure risk.",
    "Create an APXM workflow to review aam_recall output size controls and summarize context-bloat risk.",
    "Create an APXM workflow to inspect trace_fetch summary/full behavior and summarize default safety.",
    "Create an APXM workflow to review release build readiness for apxm-mcp-server across Claude and Codex.",
    "Create an APXM workflow to inspect generated docs for remaining placeholders before commit.",
    "Create an APXM workflow to summarize final ship readiness, residual risks, and verification steps.",
)


@dataclasses.dataclass(frozen=True)
class TaskOutcome:
    index: int
    trace_id: str
    task: str
    ok: bool
    elapsed_ms: int
    status: str | None
    air_hash: str | None
    artifact_hash: str | None
    stats: dict[str, Any] | None
    summary: dict[str, Any] | None
    error: str | None
    raw_response: dict[str, Any] | None


def utc_now() -> str:
    return dt.datetime.now(dt.UTC).replace(microsecond=0).isoformat().replace("+00:00", UTC_SUFFIX)


def run_id() -> str:
    return dt.datetime.now(dt.UTC).strftime("%Y%m%dT%H%M%SZ")


def default_output_dir(root: pathlib.Path, current_run_id: str) -> pathlib.Path:
    return root / ".apxm" / "evaluation" / "mcp-server" / "runs" / current_run_id


def build_request(index: int, task: str, trace_prefix: str, execute: bool) -> dict[str, Any]:
    trace_id = f"{trace_prefix}-{index:02d}"
    return {
        JsonRpcField.JSONRPC: JSONRPC_VERSION,
        JsonRpcField.ID: index,
        JsonRpcField.METHOD: JsonRpcMethod.TOOLS_CALL,
        JsonRpcField.PARAMS: {
            JsonRpcField.NAME: McpTool.PROMPT_AS_WORKFLOW,
            JsonRpcField.ARGUMENTS: {
                PlanArg.TASK: task,
                PlanArg.EXECUTE: execute,
                PlanArg.TRACE_ID: trace_id,
            },
        },
    }


def extract_jsonrpc_response(stdout: str) -> dict[str, Any]:
    for line in stdout.splitlines():
        candidate = line.strip()
        if not candidate.startswith("{"):
            continue
        try:
            parsed = json.loads(candidate)
        except json.JSONDecodeError:
            continue
        if isinstance(parsed, dict) and parsed.get(JsonRpcField.JSONRPC) == JSONRPC_VERSION:
            return parsed
    raise ValueError("command output did not contain a JSON-RPC response")


def parse_tool_text(response: dict[str, Any]) -> tuple[bool, dict[str, Any] | None, str | None]:
    result = response.get(JsonRpcField.RESULT)
    if not isinstance(result, dict):
        return False, None, "JSON-RPC response did not include a result object"

    is_error = bool(result.get(JsonRpcField.IS_ERROR))
    content = result.get(JsonRpcField.CONTENT)
    if not isinstance(content, list) or not content:
        return False, None, "MCP tool result did not include text content"
    first = content[0]
    if not isinstance(first, dict):
        return False, None, "MCP tool content item was not an object"
    text = first.get(JsonRpcField.TEXT)
    if not isinstance(text, str):
        return False, None, "MCP tool content text was missing"
    if is_error:
        return False, None, text
    try:
        parsed = json.loads(text)
    except json.JSONDecodeError as exc:
        return False, None, f"tool text was not JSON: {exc}"
    if not isinstance(parsed, dict):
        return False, None, "tool text JSON was not an object"
    return True, parsed, None


def invoke_task(
    *,
    repo: pathlib.Path,
    service: str,
    server_bin: pathlib.Path,
    request: dict[str, Any],
    timeout_seconds: int,
) -> tuple[dict[str, Any] | None, str | None]:
    command = [
        "dekk",
        "apxm",
        "vllm",
        "service-exec",
        service,
        "--",
        str(server_bin),
    ]
    try:
        completed = subprocess.run(
            command,
            cwd=repo,
            input=json.dumps(request) + "\n",
            capture_output=True,
            text=True,
            timeout=timeout_seconds,
            check=False,
        )
    except subprocess.TimeoutExpired:
        return None, f"timed out after {timeout_seconds}s"

    if completed.returncode != 0:
        stderr = completed.stderr.strip()
        stdout = completed.stdout.strip()
        return None, f"command exited {completed.returncode}: {stderr or stdout}"

    try:
        return extract_jsonrpc_response(completed.stdout), None
    except ValueError as exc:
        return None, f"{exc}: {completed.stdout.strip()}"


def run_task(
    *,
    repo: pathlib.Path,
    service: str,
    server_bin: pathlib.Path,
    index: int,
    task: str,
    trace_prefix: str,
    execute: bool,
    timeout_seconds: int,
) -> TaskOutcome:
    request = build_request(index, task, trace_prefix, execute)
    trace_id = request[JsonRpcField.PARAMS][JsonRpcField.ARGUMENTS][PlanArg.TRACE_ID]
    start = time.monotonic()
    response, command_error = invoke_task(
        repo=repo,
        service=service,
        server_bin=server_bin,
        request=request,
        timeout_seconds=timeout_seconds,
    )
    elapsed_ms = int((time.monotonic() - start) * 1000)
    if command_error:
        return TaskOutcome(
            index=index,
            trace_id=trace_id,
            task=task,
            ok=False,
            elapsed_ms=elapsed_ms,
            status=None,
            air_hash=None,
            artifact_hash=None,
            stats=None,
            summary=None,
            error=command_error,
            raw_response=response,
        )

    assert response is not None
    parsed_ok, payload, tool_error = parse_tool_text(response)
    expected_status = "executed" if execute else "compiled"
    status = payload.get(PlanResult.STATUS) if payload else None
    ok = parsed_ok and status == expected_status
    if parsed_ok and not ok:
        tool_error = f"expected status {expected_status!r}, got {status!r}"

    return TaskOutcome(
        index=index,
        trace_id=trace_id,
        task=task,
        ok=ok,
        elapsed_ms=elapsed_ms,
        status=status if isinstance(status, str) else None,
        air_hash=payload.get(PlanResult.AIR_HASH) if payload else None,
        artifact_hash=payload.get(PlanResult.ARTIFACT_HASH) if payload else None,
        stats=payload.get(PlanResult.STATS) if payload else None,
        summary=payload.get(PlanResult.SUMMARY) if payload else None,
        error=tool_error,
        raw_response=response,
    )


def write_json(path: pathlib.Path, value: Any) -> None:
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def write_jsonl(path: pathlib.Path, rows: list[TaskOutcome]) -> None:
    with path.open("w", encoding="utf-8") as handle:
        for row in rows:
            handle.write(json.dumps(dataclasses.asdict(row), sort_keys=True) + "\n")


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--service", default=DEFAULT_SERVICE)
    parser.add_argument("--server-bin", default=DEFAULT_SERVER_BIN)
    parser.add_argument("--output-dir")
    parser.add_argument("--trace-prefix")
    parser.add_argument("--limit", type=int, default=len(DOGFOOD_TASKS))
    parser.add_argument(
        "--only-index",
        action="append",
        type=int,
        help="Run only the 1-based task index. May be passed more than once.",
    )
    parser.add_argument("--timeout-seconds", type=int, default=DEFAULT_TIMEOUT_SECONDS)
    parser.add_argument("--no-execute", action="store_true")
    return parser.parse_args(argv)


def main(argv: list[str]) -> int:
    args = parse_args(argv)
    repo = pathlib.Path.cwd()
    current_run_id = run_id()
    output_dir = (
        pathlib.Path(args.output_dir)
        if args.output_dir
        else default_output_dir(repo, current_run_id)
    )
    output_dir.mkdir(parents=True, exist_ok=True)
    trace_prefix = args.trace_prefix or f"dogfood-{current_run_id}"
    server_bin = pathlib.Path(args.server_bin)
    if not server_bin.is_absolute():
        server_bin = repo / server_bin

    if args.only_index:
        selected_tasks = [
            DOGFOOD_TASKS[index - 1]
            for index in args.only_index
            if 1 <= index <= len(DOGFOOD_TASKS)
        ]
        selected_indices = [
            index for index in args.only_index if 1 <= index <= len(DOGFOOD_TASKS)
        ]
    else:
        selected_tasks = DOGFOOD_TASKS[: max(0, min(args.limit, len(DOGFOOD_TASKS)))]
        selected_indices = list(range(1, len(selected_tasks) + 1))
    execute = not args.no_execute
    started_at = utc_now()
    outcomes: list[TaskOutcome] = []
    for ordinal, (index, task) in enumerate(zip(selected_indices, selected_tasks), start=1):
        outcome = run_task(
            repo=repo,
            service=args.service,
            server_bin=server_bin,
            index=index,
            task=task,
            trace_prefix=trace_prefix,
            execute=execute,
            timeout_seconds=args.timeout_seconds,
        )
        outcomes.append(outcome)
        state = "PASS" if outcome.ok else "FAIL"
        print(
            f"{ordinal:02d}/{len(selected_tasks):02d} task={index:02d} {state} {outcome.trace_id} {outcome.elapsed_ms}ms",
            flush=True,
        )
        if outcome.error:
            print(f"  {outcome.error}", flush=True)
        write_jsonl(output_dir / "results.jsonl", outcomes)

    passed = sum(1 for outcome in outcomes if outcome.ok)
    total = len(outcomes)
    pass_rate = passed / total if total else 0.0
    summary = {
        "scenario": DEFAULT_SCENARIO,
        "started_at": started_at,
        "ended_at": utc_now(),
        "service": args.service,
        "server_bin": str(server_bin),
        "trace_prefix": trace_prefix,
        "execute": execute,
        "total": total,
        "passed": passed,
        "failed": total - passed,
        "pass_rate": pass_rate,
        "success_threshold": 0.60,
        "threshold_passed": pass_rate >= 0.60,
        "task_indices": selected_indices,
        "tasks": list(selected_tasks),
    }
    write_json(output_dir / "summary.json", summary)
    print(json.dumps(summary, indent=2, sort_keys=True), flush=True)
    return 0 if summary["threshold_passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
