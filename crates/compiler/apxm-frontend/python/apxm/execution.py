from __future__ import annotations

from dataclasses import dataclass, field
from enum import Enum
import json
import os
from pathlib import Path
import shutil
import subprocess
from typing import Any

from . import constants as graph_keys
from apxm.constants import (
    ENV_APXM_BIN,
    ENV_APXM_CONFIG,
    ENV_APXM_EMIT_AIR,
    ENV_APXM_MOCK_BACKEND,
    ENV_APXM_SERVER_URL,
    ENV_FLAG_ENABLED,
)
from .config import ExecutionOptions
from .ir import ApxmGraph
from .utils import detect_cycle
from apxm.providers import REGISTERED_PROVIDERS

_LLM_TURN_OPS = frozenset(
    {
        graph_keys.OP_ASK,
        graph_keys.OP_THINK,
        graph_keys.OP_REASON,
    }
)


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

    errors.extend(_validate_spawn_communicate_dependencies(graph))
    errors.extend(_validate_llm_operation_attributes(graph))

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


def _validate_llm_operation_attributes(graph: ApxmGraph) -> list[str]:
    errors: list[str] = []
    for node in graph.nodes:
        if graph_keys.LLM_OPERATION not in node.attributes:
            continue
        value = node.attributes[graph_keys.LLM_OPERATION]
        if not isinstance(value, str):
            errors.append(
                f"node '{node.name}' ({node.op}) has non-string "
                f"{graph_keys.LLM_OPERATION!r} attribute"
            )
            continue
        if value not in _LLM_TURN_OPS:
            valid = ", ".join(sorted(_LLM_TURN_OPS))
            errors.append(
                f"node '{node.name}' ({node.op}) has invalid "
                f"{graph_keys.LLM_OPERATION!r} value {value!r}; expected one of {valid}"
            )
    return errors


def _validate_spawn_communicate_dependencies(graph: ApxmGraph) -> list[str]:
    spawned: dict[str, int] = {}
    errors: list[str] = []

    for node in graph.nodes:
        if node.op != graph_keys.OP_SPAWN_AGENT:
            continue
        agent_name = node.attributes.get(graph_keys.AGENT_NAME)
        if isinstance(agent_name, str):
            spawned[agent_name] = node.id

    data_edges = [
        (edge.from_id, edge.to_id)
        for edge in graph.edges
        if edge.dependency == graph_keys.DEPENDENCY_DATA
    ]
    control_edges = {
        (edge.from_id, edge.to_id)
        for edge in graph.edges
        if edge.dependency == graph_keys.DEPENDENCY_CONTROL
    }

    for node in graph.nodes:
        if node.op != graph_keys.OP_COMMUNICATE:
            continue
        recipient = node.attributes.get(graph_keys.RECIPIENT)
        if not isinstance(recipient, str) or recipient not in spawned:
            continue
        spawn_id = spawned[recipient]
        message_input_count = _input_names_count(node.attributes.get(graph_keys.INPUT_NAMES))
        data_sources = [
            from_id for from_id, to_id in data_edges if to_id == node.id
        ]
        structural_sources = data_sources[message_input_count:]
        if any(_has_data_path(spawn_id, source_id, data_edges) for source_id in structural_sources):
            continue
        if (spawn_id, node.id) in control_edges:
            hint = (
                "Control edges do not carry the spawn token into AIR operands"
            )
        else:
            hint = "no Data dependency path from the matching SPAWN_AGENT was found"
        errors.append(
            f"node '{node.name}' ({node.op}) targets spawned agent '{recipient}' "
            f"but does not depend on its SPAWN_AGENT token: {hint}"
        )

    return errors


def _has_data_path(
    source_id: int,
    target_id: int,
    data_edges: list[tuple[int, int]],
) -> bool:
    adjacency: dict[int, list[int]] = {}
    for from_id, to_id in data_edges:
        adjacency.setdefault(from_id, []).append(to_id)

    visited: set[int] = set()
    pending = [source_id]
    while pending:
        current = pending.pop()
        if current in visited:
            continue
        visited.add(current)
        for next_id in adjacency.get(current, []):
            if next_id == target_id:
                return True
            pending.append(next_id)
    return False


def _input_names_count(value: Any) -> int:
    if isinstance(value, list):
        return len(value)
    if isinstance(value, str):
        return 1
    return 0


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
    execution_id: str | None = None
    session_dir: str | None = None
    metrics_path: str | None = None
    profile_path: str | None = None
    results: dict[str, Any] = field(default_factory=dict)
    stats: ExecutionStats = field(default_factory=ExecutionStats)
    llm_usage: LLMUsage = field(default_factory=LLMUsage)

    @classmethod
    def from_response(cls, data: dict[str, Any]) -> ExecutionResult:
        if not isinstance(data, dict):
            raise TypeError("ExecutionResult response must be an object")

        execution_id = data.get("execution_id")
        if execution_id is not None and not isinstance(execution_id, str):
            raise TypeError("ExecutionResult.execution_id must be a string or null")

        session_dir = data.get("session_dir")
        if session_dir is not None and not isinstance(session_dir, str):
            raise TypeError("ExecutionResult.session_dir must be a string or null")

        metrics_path = data.get("metrics_path")
        if metrics_path is not None and not isinstance(metrics_path, str):
            raise TypeError("ExecutionResult.metrics_path must be a string or null")

        profile_path = data.get("profile_path")
        if profile_path is not None and not isinstance(profile_path, str):
            raise TypeError("ExecutionResult.profile_path must be a string or null")

        results = data.get("results", {})
        if not isinstance(results, dict):
            raise TypeError("ExecutionResult.results must be an object")

        stats = data.get(
            "stats",
            {
                "executed_nodes": 0,
                "failed_nodes": 0,
                "duration_ms": 0,
            },
        )
        if not isinstance(stats, dict):
            raise TypeError("ExecutionResult.stats must be an object")

        llm_usage = data.get(
            "llm_usage",
            {
                "input_tokens": 0,
                "output_tokens": 0,
                "total_requests": 0,
            },
        )
        if not isinstance(llm_usage, dict):
            raise TypeError("ExecutionResult.llm_usage must be an object")

        return cls(
            content=data.get("content"),
            execution_id=execution_id,
            session_dir=session_dir,
            metrics_path=metrics_path,
            profile_path=profile_path,
            results=results,
            stats=ExecutionStats(**stats),
            llm_usage=LLMUsage(**llm_usage),
        )


@dataclass(slots=True)
class WorkflowRunResult:
    stdout: str
    session_dir: str | None = None


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

_CLI_APXM_WRAPPER_SUBCOMMAND = "apxm"
_CLI_DEKK_BINARY = "dekk"
_CLI_APXM_BINARY = "apxm"
_CLI_CONFIG_FLAG = "--config"
_CLI_JSON_FLAG = "--json"
_CLI_EXECUTE_SUBCOMMAND = "execute"
_CLI_WORKFLOW_SUBCOMMAND = "workflow"
_CLI_RUN_SUBCOMMAND = "run"
_CLI_EMIT_SESSION_FLAG = "--emit-session"
_CLI_SESSION_ROOT_FLAG = "--session-root"
_CLI_ARGS_JSON_FLAG = "--args-json"
_CLI_BINARY_ENV = ENV_APXM_BIN
_CARGO_TARGET_DIR = "target"
_CARGO_RELEASE_PROFILE = "release"
_CARGO_DEBUG_PROFILE = "debug"
_DEFAULT_SERVER_URL = "http://localhost:18800"
_APXM_DIR_NAME = ".apxm"
_APXM_CONFIG_FILE_NAME = "config.toml"


def _server_url() -> str:
    return os.environ.get(ENV_APXM_SERVER_URL, _DEFAULT_SERVER_URL)


def emit_air_if_requested(flow: Any) -> bool:
    """Emit a compiled Python graph for `dekk apxm compile` and return true."""

    if os.environ.get(ENV_APXM_EMIT_AIR) != ENV_FLAG_ENABLED:
        return False
    air_text = getattr(flow, "_air_text", None)
    if air_text is None:
        graph = getattr(flow, "_graph", None)
        air_text = graph.to_air() if graph is not None else None
    if air_text is None:
        raise TypeError("emit_air_if_requested expects a compiled APXM flow")
    print(air_text)
    return True


def _build_cli_base_command(apxm_bin: str) -> list[str]:
    if Path(apxm_bin).name == _CLI_DEKK_BINARY:
        return [apxm_bin, _CLI_APXM_WRAPPER_SUBCOMMAND]
    return [apxm_bin]


def _append_session_root_flag(
    cmd: list[str],
    *,
    target: str,
    session_root: str | os.PathLike[str] | None,
) -> None:
    if session_root is None:
        return
    if target == _CLI_EXECUTE_SUBCOMMAND:
        cmd.extend([_CLI_EMIT_SESSION_FLAG, os.fspath(session_root)])
        return
    if target == _CLI_WORKFLOW_SUBCOMMAND:
        cmd.extend([_CLI_SESSION_ROOT_FLAG, os.fspath(session_root)])
        return
    raise ValueError(f"unsupported session-root target: {target}")


def _cli_error_message(stdout: str, stderr: str, default_message: str) -> str:
    try:
        payload = json.loads(stdout)
    except json.JSONDecodeError:
        payload = None

    if isinstance(payload, dict):
        error = payload.get("error")
        if isinstance(error, str) and error.strip():
            return error.strip()

    for stream in (stderr, stdout):
        text = stream.strip()
        if text:
            return text

    return default_message


def _subprocess_env_for_apxm(apxm_bin: str) -> dict[str, str]:
    env = os.environ.copy()
    binary_path = Path(apxm_bin).resolve()
    if (
        binary_path.parent.name in (_CARGO_RELEASE_PROFILE, _CARGO_DEBUG_PROFILE)
        and binary_path.parent.parent.name == _CARGO_TARGET_DIR
    ):
        lib_dir = binary_path.parent / "lib"
        if lib_dir.is_dir():
            _prepend_env_path(env, "LD_LIBRARY_PATH", str(lib_dir))
            _prepend_env_path(env, "DYLD_LIBRARY_PATH", str(lib_dir))
    return env


def _active_apxm_config_path() -> Path | None:
    explicit = os.environ.get(ENV_APXM_CONFIG, "").strip()
    if explicit:
        return Path(explicit)

    cwd = Path.cwd().resolve()
    for candidate_root in (cwd, *cwd.parents):
        candidate = candidate_root / _APXM_DIR_NAME / _APXM_CONFIG_FILE_NAME
        if candidate.is_file():
            return candidate

    home = Path.home() / _APXM_DIR_NAME / _APXM_CONFIG_FILE_NAME
    return home if home.is_file() else None


def _execution_overlay_config_toml(overlay_toml: str) -> str:
    active_path = _active_apxm_config_path()
    if active_path is None:
        return overlay_toml

    base_toml = active_path.read_text()
    if not base_toml.strip():
        return overlay_toml
    if not overlay_toml.strip():
        return base_toml
    return base_toml.rstrip() + "\n\n" + overlay_toml


def _prepend_env_path(env: dict[str, str], key: str, path: str) -> None:
    current = env.get(key)
    if not current:
        env[key] = path
        return
    parts = current.split(os.pathsep)
    if path in parts:
        return
    env[key] = path + os.pathsep + current


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
    """Locate a direct APXM CLI binary for machine-driven local execution."""
    explicit = os.environ.get(_CLI_BINARY_ENV)
    if explicit:
        explicit_path = Path(explicit)
        if explicit_path.is_file():
            return str(explicit_path)
        explicit_bin = shutil.which(explicit)
        if explicit_bin is not None:
            return explicit_bin
        raise RuntimeError(
            f"{_CLI_BINARY_ENV} points to a missing APXM binary: {explicit_path}"
        )

    apxm_bin = shutil.which(_CLI_APXM_BINARY)
    if apxm_bin is not None:
        return apxm_bin

    checkout_bin = _find_checkout_apxm_binary()
    if checkout_bin is not None:
        return str(checkout_bin)

    dekk_bin = shutil.which(_CLI_DEKK_BINARY)
    if dekk_bin is not None:
        return dekk_bin

    raise RuntimeError(
        f"No direct 'apxm' binary found. Set {ENV_APXM_BIN}, install 'apxm' on PATH, "
        "install 'dekk' on PATH, or build the repo-local APXM binary."
    )


def _find_checkout_apxm_binary() -> Path | None:
    seen: set[Path] = set()
    for start in (Path.cwd(), Path(__file__).resolve()):
        for ancestor in (start, *start.parents):
            if ancestor in seen:
                continue
            seen.add(ancestor)
            for profile in (_CARGO_RELEASE_PROFILE, _CARGO_DEBUG_PROFILE):
                candidate = ancestor / _CARGO_TARGET_DIR / profile / _CLI_APXM_BINARY
                if candidate.is_file():
                    return candidate
    return None


# ---------------------------------------------------------------------------
# CompiledFlow
# ---------------------------------------------------------------------------

class CompiledFlow:
    __slots__ = ("_graph", "mode", "_opt_level", "_registered_tools", "_air_text")

    def __init__(
        self,
        graph: ApxmGraph,
        *,
        mode: ExecutionMode = ExecutionMode.COMPILED,
        opt_level: int | None = None,
        air_text: str | None = None,
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
        self._air_text = air_text

    # -- persistence --------------------------------------------------------

    def save(self, path: str | os.PathLike[str]) -> None:
        envelope = {
            graph_keys.AIR_PAYLOAD: self._air_text if self._air_text else self._graph.to_air(),
            "mode": self.mode.value,
            "opt_level": self._opt_level,
        }
        Path(path).write_text(json.dumps(envelope, indent=2), encoding="utf-8")

    @classmethod
    def load(cls, path: str | os.PathLike[str]) -> "CompiledFlow":
        path = Path(path)
        text = path.read_text(encoding="utf-8")
        data = json.loads(text)
        mode = ExecutionMode(data.get("mode", ExecutionMode.AOT.value))
        opt_level = int(data.get("opt_level", 2))
        air_text = data[graph_keys.AIR_PAYLOAD]
        return cls(ApxmGraph(name=path.stem), mode=mode, opt_level=opt_level, air_text=air_text)

    def register_tool(self, tool: Any) -> "CompiledFlow":
        self._registered_tools.append(tool)
        return self

    # -- execution ----------------------------------------------------------

    async def run(
        self,
        *args: Any,
        session_id: str | None = None,
        execution: ExecutionOptions | None = None,
    ) -> ExecutionResult:
        from .errors import CompilationError, ServerError

        if self._air_text is None:
            errors = validate_graph(self._graph)
            if errors:
                raise CompilationError("invalid graph: " + "; ".join(errors))

        execution = _merge_execution_options(session_id, execution)
        request_body = self._build_request(args, execution=execution)

        if execution.requires_local_cli():
            if execution.session_id is not None:
                raise ServerError(
                    "session_id is only supported by the HTTP server path; "
                    "remove local-only execution options or unset session_id"
                )
            return self._fallback_subprocess(*args, execution=execution)

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
        if execution.session_id is not None:
            raise ServerError(
                "session_id requires the HTTP server. "
                "Start it with: dekk apxm server"
            )
        return self._fallback_subprocess(*args, execution=execution)

    def run_sync(
        self,
        *args: Any,
        session_id: str | None = None,
        execution: ExecutionOptions | None = None,
    ) -> ExecutionResult:
        """Synchronous execution convenience method."""
        import asyncio
        import threading

        result: ExecutionResult | None = None
        exc: BaseException | None = None

        def _runner() -> None:
            nonlocal result, exc
            try:
                result = asyncio.run(self.run(*args, session_id=session_id, execution=execution))
            except BaseException as e:
                exc = e

        t = threading.Thread(target=_runner)
        t.start()
        t.join()
        if exc is not None:
            raise exc
        assert result is not None
        return result

    async def stream(
        self,
        *args: Any,
        session_id: str | None = None,
        execution: ExecutionOptions | None = None,
    ):
        """Async generator yielding execution events via SSE."""
        from .errors import CompilationError, ServerError

        if self._air_text is None:
            errors = validate_graph(self._graph)
            if errors:
                raise CompilationError("invalid graph: " + "; ".join(errors))

        execution = _merge_execution_options(session_id, execution)
        if execution.requires_local_cli():
            raise ServerError(
                "stream() requires the HTTP server path and does not support "
                "local-only execution options like hooks or middlewares"
            )

        try:
            import httpx
            from httpx_sse import aconnect_sse
        except ImportError:
            raise RuntimeError(
                "httpx and httpx-sse are required for streaming. "
                "Install with: pip install apxm[stream]"
            )

        client = await _get_client()
        request_body = self._build_request(args, execution=execution)

        async with aconnect_sse(
            client, "POST", "/v1/execute/stream",
            json=request_body,
            timeout=httpx.Timeout(connect=5.0, read=None, write=10.0, pool=5.0),
        ) as source:
            async for sse in source.aiter_sse():
                yield json.loads(sse.data)

    # -- internal helpers ---------------------------------------------------

    def _build_request(
        self,
        args: tuple[Any, ...],
        *,
        execution: ExecutionOptions | None = None,
    ) -> dict[str, Any]:
        execution = execution or ExecutionOptions()
        runtime_graph = _graph_with_execution_overrides(self._graph, execution)
        if runtime_graph is self._graph:
            air_text = self._air_text if self._air_text else self._graph.to_air()
        else:
            air_text = runtime_graph.to_air()
        request: dict[str, Any] = {
            graph_keys.AIR_PAYLOAD: air_text,
            "args": [str(a) for a in args],
        }
        request.update(execution.server_request_fields())
        return request

    def _fallback_subprocess(
        self,
        *args: Any,
        execution: ExecutionOptions | None = None,
    ) -> ExecutionResult:
        from .errors import ExecutionError
        import tempfile

        apxm_bin = _find_apxm_binary()
        execution = execution or ExecutionOptions()

        # Use pre-captured AIR text when available; otherwise fall back
        # to ApxmGraph.to_air(). Strip the sidecar comment (if present)
        # because the MLIR parser does not understand `;` comments.
        runtime_graph = _graph_with_execution_overrides(self._graph, execution)
        if runtime_graph is self._graph:
            air_text = self._air_text if self._air_text else self._graph.to_air()
        else:
            air_text = runtime_graph.to_air()
            if self._air_text:
                sidecar_lines = [
                    line
                    for line in self._air_text.splitlines()
                    if line.startswith("; __apxm_python_tools__")
                ]
                if sidecar_lines:
                    air_text = "\n".join(sidecar_lines + [air_text])
        clean_lines = [
            line for line in air_text.splitlines()
            if not line.startswith("; __apxm_python_tools__")
        ]
        clean_air = "\n".join(clean_lines)

        # Write AIR text to a tempfile (.air) so the compiler can parse it
        # directly. Using AIR preserves the graph form the runtime executes,
        # including named parameter references and operation attributes.
        with tempfile.NamedTemporaryFile(
            mode="w", suffix=".air", delete=False
        ) as tmp:
            tmp.write(clean_air)
            tmp_path = tmp.name

        config_path: str | None = None
        config_toml = execution.config_toml()
        if config_toml:
            with tempfile.NamedTemporaryFile(
                mode="w", suffix=".toml", delete=False
            ) as tmp:
                tmp.write(_execution_overlay_config_toml(config_toml))
                config_path = tmp.name

        try:
            cmd = _build_cli_base_command(apxm_bin)

            if config_path is not None:
                cmd.extend([_CLI_CONFIG_FLAG, config_path])

            cmd.extend([_CLI_JSON_FLAG, _CLI_EXECUTE_SUBCOMMAND, tmp_path])
            _append_session_root_flag(
                cmd,
                target=_CLI_EXECUTE_SUBCOMMAND,
                session_root=execution.session_root,
            )

            for arg in args:
                cmd.append(str(arg))

            result = subprocess.run(
                cmd,
                capture_output=True,
                text=True,
                check=False,
                env=_subprocess_env_for_apxm(apxm_bin),
            )
        finally:
            Path(tmp_path).unlink(missing_ok=True)
            if config_path is not None:
                Path(config_path).unlink(missing_ok=True)

        if result.returncode != 0:
            raise ExecutionError(
                f"apxm execute failed (exit {result.returncode}): "
                f"{_cli_error_message(result.stdout, result.stderr, 'unknown cli error')}"
            )

        try:
            payload = json.loads(result.stdout)
        except json.JSONDecodeError as exc:
            raise ExecutionError(
                "apxm execute --json returned invalid JSON output"
            ) from exc

        if not isinstance(payload, dict):
            raise ExecutionError(
                "apxm execute --json returned a non-object payload"
            )

        try:
            return ExecutionResult.from_response(payload)
        except (TypeError, ValueError, AttributeError) as exc:
            raise ExecutionError(
                "apxm execute --json returned an unexpected response shape"
            ) from exc


def _merge_execution_options(
    session_id: str | None,
    execution: ExecutionOptions | None,
) -> ExecutionOptions:
    if execution is None:
        return ExecutionOptions(session_id=session_id)
    if session_id is not None and execution.session_id not in (None, session_id):
        raise ValueError("session_id and execution.session_id must match when both are set")
    if session_id is not None and execution.session_id is None:
        execution = ExecutionOptions(
            session_id=session_id,
            session_root=execution.session_root,
            token_budget=execution.token_budget,
            output_schema=execution.output_schema,
            max_schema_retries=execution.max_schema_retries,
            hooks=list(execution.hooks),
            middlewares=list(execution.middlewares),
        )
    return execution


def _parse_workflow_run_output(stdout: str) -> WorkflowRunResult:
    try:
        payload = json.loads(stdout)
    except json.JSONDecodeError as exc:
        raise ValueError(
            "apxm workflow run --json returned invalid JSON output"
        ) from exc
    if not isinstance(payload, dict):
        raise ValueError("apxm workflow run --json returned a non-object payload")
    session_dir = payload.get("session_dir")
    if session_dir is not None and not isinstance(session_dir, str):
        raise ValueError("apxm workflow run --json returned a non-string session_dir")
    return WorkflowRunResult(stdout=stdout, session_dir=session_dir)


def run_workflow_file(
    path: str | os.PathLike[str],
    *,
    args: dict[str, Any] | None = None,
    session_root: str | os.PathLike[str] | None = None,
) -> WorkflowRunResult:
    from .errors import ExecutionError

    apxm_bin = _find_apxm_binary()
    cmd = _build_cli_base_command(apxm_bin)
    cmd.extend(
        [_CLI_JSON_FLAG, _CLI_WORKFLOW_SUBCOMMAND, _CLI_RUN_SUBCOMMAND, os.fspath(path)]
    )
    _append_session_root_flag(
        cmd,
        target=_CLI_WORKFLOW_SUBCOMMAND,
        session_root=session_root,
    )
    if args:
        cmd.extend([_CLI_ARGS_JSON_FLAG, json.dumps(args)])

    result = subprocess.run(
        cmd,
        capture_output=True,
        text=True,
        check=False,
        env=_subprocess_env_for_apxm(apxm_bin),
    )
    if result.returncode != 0:
        raise ExecutionError(
            f"apxm workflow run failed (exit {result.returncode}): "
            f"{_cli_error_message(result.stdout, result.stderr, 'unknown cli error')}"
        )

    try:
        return _parse_workflow_run_output(result.stdout)
    except ValueError as exc:
        raise ExecutionError(str(exc)) from exc


def _graph_with_execution_overrides(
    graph: ApxmGraph,
    execution: ExecutionOptions,
) -> ApxmGraph:
    if (
        execution.token_budget is None
        and execution.output_schema is None
        and execution.max_schema_retries is None
    ):
        return graph

    graph_copy = ApxmGraph.from_dict(graph.to_dict())
    llm_ops = getattr(graph_keys, "LLM_OPS", frozenset())
    for node in graph_copy.nodes:
        if node.op not in llm_ops:
            continue
        if execution.token_budget is not None:
            node.attributes[graph_keys.TOKEN_BUDGET] = execution.token_budget
        if execution.output_schema is not None:
            node.attributes[graph_keys.OUTPUT_SCHEMA] = execution.output_schema
        if execution.max_schema_retries is not None:
            node.attributes[graph_keys.MAX_SCHEMA_RETRIES] = execution.max_schema_retries
    return graph_copy


def run(coro, *args, mock: bool = False, **kwargs: Any):
    """Execute an async workflow and return the result.

    When called with a single coroutine argument (e.g. ``run(flow())``) this is
    a thin wrapper around ``asyncio.run()``.

    When called with a compiled flow and positional arguments
    (e.g. ``run(math_flow, "What is 17 + 25?")``) it invokes the flow with
    those args.

    If *mock* is ``True`` (or the APXM mock backend environment flag is
    already set) the subprocess child will have the mock flag enabled so the
    runtime uses the deterministic mock backend — no real API keys required.
    """
    import asyncio

    if mock:
        os.environ[ENV_APXM_MOCK_BACKEND] = ENV_FLAG_ENABLED

    # If `coro` is a compiled flow (callable), invoke it with the args first.
    if callable(coro) and not asyncio.iscoroutine(coro):
        coro = coro(*args, **kwargs)

    return asyncio.run(coro)
