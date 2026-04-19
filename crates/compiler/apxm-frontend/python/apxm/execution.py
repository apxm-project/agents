from __future__ import annotations

from dataclasses import dataclass, field
from enum import Enum
import json
import os
from pathlib import Path
import shutil
import subprocess
from typing import Any

from apxm._generated import constants as graph_keys
from .ir import ApxmGraph
from .utils import detect_cycle
from apxm.providers import REGISTERED_PROVIDERS


class ExecutionMode(Enum):
    EAGER = "eager"
    COMPILED = "compiled"
    AOT = "aot"


def validate_graph(graph: ApxmGraph) -> list[str]:
    errors: list[str] = []

    if not graph.name.strip():
        errors.append("graph name cannot be empty")

    node_ids = [node.id for node in graph.nodes]
    seen_ids: set[int] = set()
    for node_id in node_ids:
        if node_id in seen_ids:
            errors.append(f"duplicate node id: {node_id}")
        seen_ids.add(node_id)

    for edge in graph.edges:
        if edge.from_id not in seen_ids:
            errors.append(f"edge references unknown source node {edge.from_id}")
        if edge.to_id not in seen_ids:
            errors.append(f"edge references unknown destination node {edge.to_id}")

    edges = [
        (edge.from_id, edge.to_id)
        for edge in graph.edges
        if edge.from_id in seen_ids and edge.to_id in seen_ids
    ]
    cycle_count = detect_cycle(seen_ids, edges)
    if cycle_count:
        errors.append("graph contains a cycle")

    # Validate providers on LLM nodes
    for node in graph.nodes:
        if node.op not in getattr(graph_keys, "LLM_OPS", frozenset()):
            continue
        provider = node.attributes.get(graph_keys.PROVIDER)
        if provider is None:
            continue
        if provider.lower() not in REGISTERED_PROVIDERS:
            from apxm.providers import list_providers
            valid = ", ".join(list_providers())
            errors.append(
                f"node '{node.name}' ({node.op}): unknown provider '{provider}'. "
                f"Registered providers: {valid}"
            )

    return errors


@dataclass(slots=True)
class WorkflowCheckpoint:
    """Serializable snapshot of workflow execution state.

    Captures completed node outputs and pending nodes so that a workflow
    can be paused at a checkpoint/fence and resumed later.
    """

    artifact_bytes: bytes = b""
    completed_nodes: dict[str, Any] = None  # type: ignore[assignment]
    pending_nodes: list[int] = None  # type: ignore[assignment]
    metadata: dict[str, Any] = None  # type: ignore[assignment]

    def __post_init__(self) -> None:
        if self.completed_nodes is None:
            self.completed_nodes = {}
        if self.pending_nodes is None:
            self.pending_nodes = []
        if self.metadata is None:
            self.metadata = {}

    def save(self, path: str | os.PathLike[str]) -> None:
        """Persist the checkpoint to a JSON file."""
        envelope = {
            "artifact_bytes": list(self.artifact_bytes),
            "completed_nodes": self.completed_nodes,
            "pending_nodes": self.pending_nodes,
            "metadata": self.metadata,
        }
        Path(path).write_text(json.dumps(envelope, indent=2), encoding="utf-8")

    @classmethod
    def load(cls, path: str | os.PathLike[str]) -> "WorkflowCheckpoint":
        """Load a checkpoint from a JSON file."""
        data = json.loads(Path(path).read_text(encoding="utf-8"))
        return cls(
            artifact_bytes=bytes(data.get("artifact_bytes", [])),
            completed_nodes=data.get("completed_nodes", {}),
            pending_nodes=data.get("pending_nodes", []),
            metadata=data.get("metadata", {}),
        )


# ---------------------------------------------------------------------------
# Execution result types
# ---------------------------------------------------------------------------

@dataclass(slots=True)
class ExecutionStats:
    executed_nodes: int = 0
    failed_nodes: int = 0
    duration_ms: int = 0


@dataclass(slots=True)
class LLMUsage:
    input_tokens: int = 0
    output_tokens: int = 0
    total_requests: int = 0


@dataclass(slots=True)
class ExecutionResult:
    content: str | None = None
    results: dict[str, Any] = field(default_factory=dict)
    stats: ExecutionStats = field(default_factory=ExecutionStats)
    llm_usage: LLMUsage = field(default_factory=LLMUsage)

    @classmethod
    def from_response(cls, data: dict[str, Any]) -> ExecutionResult:
        return cls(
            content=data.get("content"),
            results=data.get("results", {}),
            stats=ExecutionStats(
                **data.get("stats", {
                    "executed_nodes": 0,
                    "failed_nodes": 0,
                    "duration_ms": 0,
                }),
            ),
            llm_usage=LLMUsage(
                **data.get("llm_usage", {
                    "input_tokens": 0,
                    "output_tokens": 0,
                    "total_requests": 0,
                }),
            ),
        )


# ---------------------------------------------------------------------------
# Session helper
# ---------------------------------------------------------------------------

def new_session() -> str:
    """Generate a new session ID for linking multiple executions."""
    import uuid
    return f"s-{uuid.uuid4()}"


# ---------------------------------------------------------------------------
# HTTP client management
# ---------------------------------------------------------------------------

_client: Any = None  # httpx.AsyncClient | None


def _server_url() -> str:
    return os.environ.get("APXM_SERVER_URL", "http://localhost:18800")


async def _get_client() -> Any:
    global _client
    if _client is None:
        try:
            import httpx
        except ImportError:
            raise RuntimeError(
                "httpx is required for remote execution. "
                "Install it with: pip install apxm[server]"
            )
        _client = httpx.AsyncClient(
            base_url=_server_url(),
            timeout=httpx.Timeout(connect=5.0, read=300.0, write=10.0, pool=5.0),
        )
    return _client


async def close() -> None:
    """Close the shared HTTP client."""
    global _client
    if _client is not None:
        await _client.aclose()
        _client = None


# ---------------------------------------------------------------------------
# Binary lookup
# ---------------------------------------------------------------------------

def _find_apxm_binary() -> str:
    """Locate the APXM CLI, preferring the dekk wrapper when available."""
    dekk_bin = shutil.which("dekk")
    if dekk_bin is not None:
        return dekk_bin

    apxm_bin = shutil.which("apxm")
    if apxm_bin is not None:
        return apxm_bin

    raise RuntimeError(
        "Neither 'dekk' nor 'apxm' found on PATH. Install APXM with: dekk apxm install"
    )


# ---------------------------------------------------------------------------
# CompiledFlow
# ---------------------------------------------------------------------------

class CompiledFlow:
    __slots__ = ("_graph", "mode", "_opt_level", "_registered_tools")

    def __init__(
        self,
        graph: ApxmGraph,
        *,
        mode: ExecutionMode = ExecutionMode.COMPILED,
        opt_level: int | None = None,
    ) -> None:
        self._graph = graph
        self.mode = mode
        if opt_level is not None:
            self._opt_level = opt_level
        elif mode is ExecutionMode.EAGER:
            self._opt_level = 0
        else:
            self._opt_level = 2
        self._registered_tools: list[Any] = []

    # -- persistence --------------------------------------------------------

    def save(self, path: str | os.PathLike[str]) -> None:
        envelope = {
            graph_keys.GRAPH_PAYLOAD: self._graph.to_dict(),
            "mode": self.mode.value,
            "opt_level": self._opt_level,
        }
        Path(path).write_text(json.dumps(envelope, indent=2), encoding="utf-8")

    @classmethod
    def load(cls, path: str | os.PathLike[str]) -> "CompiledFlow":
        path = Path(path)
        text = path.read_text(encoding="utf-8")
        data = json.loads(text)
        graph = ApxmGraph.from_dict(data[graph_keys.GRAPH_PAYLOAD])
        mode = ExecutionMode(data.get("mode", ExecutionMode.AOT.value))
        opt_level = int(data.get("opt_level", 2))
        return cls(graph, mode=mode, opt_level=opt_level)

    def register_tool(self, tool: Any) -> "CompiledFlow":
        self._registered_tools.append(tool)
        return self

    # -- execution ----------------------------------------------------------

    async def run(self, *args: Any, session_id: str | None = None) -> ExecutionResult:
        from .errors import CompilationError, ExecutionError, ServerError

        errors = validate_graph(self._graph)
        if errors:
            raise CompilationError("invalid graph: " + "; ".join(errors))

        request_body = self._build_request(args, session_id=session_id)

        # Try HTTP first
        try:
            client = await _get_client()
            response = await client.post("/v1/execute", json=request_body)
            if response.status_code != 200:
                raise ServerError(
                    f"Server returned {response.status_code}: {response.text}"
                )
            return ExecutionResult.from_response(response.json())
        except RuntimeError:
            # httpx not installed, fall through to subprocess
            pass
        except Exception as exc:
            # Connection refused or other HTTP error -- try subprocess
            if "httpx" in type(exc).__module__:
                pass
            else:
                raise

        # Subprocess fallback (stdin, no temp files)
        if session_id is not None:
            raise ServerError(
                "session_id requires the HTTP server. "
                "Start it with: dekk apxm server"
            )
        return self._fallback_subprocess(*args)

    def run_sync(self, *args: Any, session_id: str | None = None) -> ExecutionResult:
        """Synchronous execution convenience method."""
        import asyncio
        import threading

        result: ExecutionResult | None = None
        exc: BaseException | None = None

        def _runner() -> None:
            nonlocal result, exc
            try:
                result = asyncio.run(self.run(*args, session_id=session_id))
            except BaseException as e:
                exc = e

        t = threading.Thread(target=_runner)
        t.start()
        t.join()
        if exc is not None:
            raise exc
        assert result is not None
        return result

    async def stream(self, *args: Any, session_id: str | None = None):
        """Async generator yielding execution events via SSE."""
        from .errors import CompilationError, ServerError

        errors = validate_graph(self._graph)
        if errors:
            raise CompilationError("invalid graph: " + "; ".join(errors))

        try:
            import httpx
            from httpx_sse import aconnect_sse
        except ImportError:
            raise RuntimeError(
                "httpx and httpx-sse are required for streaming. "
                "Install with: pip install apxm[stream]"
            )

        client = await _get_client()
        request_body = self._build_request(args, session_id=session_id)

        async with aconnect_sse(
            client, "POST", "/v1/execute/stream",
            json=request_body,
            timeout=httpx.Timeout(connect=5.0, read=None, write=10.0, pool=5.0),
        ) as source:
            async for sse in source.aiter_sse():
                yield json.loads(sse.data)

    # -- internal helpers ---------------------------------------------------

    def _build_request(
        self, args: tuple[Any, ...], *, session_id: str | None = None
    ) -> dict[str, Any]:
        request: dict[str, Any] = {
            "graph": self._graph.to_dict(),
            "args": [str(a) for a in args],
        }
        if session_id is not None:
            request["session_id"] = session_id
        return request

    def _fallback_subprocess(self, *args: Any) -> ExecutionResult:
        from .errors import ExecutionError

        apxm_bin = _find_apxm_binary()
        graph_json = json.dumps(self._graph.to_dict())

        if Path(apxm_bin).name == "dekk":
            cmd = [apxm_bin, "apxm", "execute", "/dev/stdin", "--json"]
        else:
            cmd = [apxm_bin, "execute", "/dev/stdin", "--json"]

        for arg in args:
            cmd.append(str(arg))

        result = subprocess.run(
            cmd, input=graph_json, capture_output=True, text=True, check=False
        )

        if result.returncode != 0:
            raise ExecutionError(
                f"apxm execute failed (exit {result.returncode}): {result.stderr.strip()}"
            )

        stdout = result.stdout.strip()
        if not stdout:
            return ExecutionResult()
        try:
            return ExecutionResult.from_response(json.loads(stdout))
        except (json.JSONDecodeError, TypeError):
            return ExecutionResult(content=stdout)


def run(coro):
    """Execute an async workflow and return the result.

    Convenience wrapper around asyncio.run() for executing compiled workflows.
    """
    import asyncio
    return asyncio.run(coro)
