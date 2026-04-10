from __future__ import annotations

from collections.abc import Generator
from dataclasses import dataclass
from enum import Enum
import importlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
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


def _serialize_args(args: tuple[Any, ...]) -> str | None:
    if not args:
        return None
    return str(args[0]) if len(args) == 1 else json.dumps(args)


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


def _build_graph_text(graph: ApxmGraph, graph_format: str) -> str:
    if graph_format == "air":
        return graph.to_air()
    if graph_format == "json":
        return graph.to_json(indent=2)
    raise ValueError(f"unsupported graph format '{graph_format}'")


def _fallback_retry_reason(stderr: str) -> bool:
    lowered = stderr.lower()
    return any(
        marker in lowered
        for marker in (
            ".air is an inspection",
            "not a compile input",
            "round-trip compilable",
            "use json graphs for compilation and execution instead",
            "use a json graph instead",
        )
    )


class CompiledFlow:
    __slots__ = ("_graph", "mode", "_opt_level", "_compiled_native", "_registered_tools")

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
        self._compiled_native: Any = None
        self._registered_tools: list[Any] = []

    async def run(self, *args: Any) -> Any:
        errors = validate_graph(self._graph)
        if errors:
            raise ValueError("invalid graph: " + "; ".join(errors))

        if await self._ensure_native_compiled():
            payload = _serialize_args(args) or ""
            return await self._compiled_native.run(payload)

        return self._fallback_subprocess(*args)

    def save(self, path: str | os.PathLike[str]) -> None:
        path = Path(path)

        if self._compiled_native is not None and hasattr(self._compiled_native, "save"):
            self._compiled_native.save(str(path))
            return

        envelope = {
            graph_keys.GRAPH_PAYLOAD: self._graph.to_dict(),
            "mode": self.mode.value,
            "opt_level": self._opt_level,
        }
        path.write_text(json.dumps(envelope, indent=2), encoding="utf-8")

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

    async def _ensure_native_compiled(self) -> bool:
        if self._compiled_native is not None:
            return True

        try:
            native = importlib.import_module("apxm")
        except Exception:
            return False

        builder_cls = getattr(native, "WorkflowBuilder", None)
        if builder_cls is None:
            return False

        if hasattr(builder_cls, "from_graph_air"):
            builder = builder_cls.from_graph_air(self._graph.to_air())
        elif hasattr(builder_cls, "from_graph_json"):
            builder = builder_cls.from_graph_json(self._graph.to_json(indent=0))
        else:
            return False

        for tool in self._registered_tools:
            schema = tool.schema() if callable(tool.schema) else tool.schema
            builder.with_tool(
                name=tool.name,
                description=tool.description,
                schema_json=json.dumps(schema),
                invoke_fn=tool.invoke,
            )

        self._compiled_native = await builder.compile(self._opt_level)
        return True

    def run_streaming(self, *args: Any) -> Generator[dict[str, Any], None, None]:
        errors = validate_graph(self._graph)
        if errors:
            yield {"event": "error", "message": "invalid graph: " + "; ".join(errors)}
            return

        yield {"event": "validation_ok"}

        try:
            apxm_bin = _find_apxm_binary()
        except RuntimeError as e:
            yield {"event": "error", "message": str(e)}
            return

        for node in self._graph.nodes:
            yield {"event": "node_started", "node_id": node.id, "name": node.name}

        attempted_formats = ("air", "json")
        for graph_format in attempted_formats:
            cmd, tmp_path = self._build_subprocess_cmd(apxm_bin, args, graph_format=graph_format)
            try:
                proc = subprocess.Popen(
                    cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True
                )

                for line in proc.stdout:  # type: ignore[union-attr]
                    line = line.strip()
                    if not line:
                        continue
                    try:
                        parsed = json.loads(line)
                        yield {"event": "output", "data": parsed}
                    except json.JSONDecodeError:
                        yield {"event": "output", "data": line}

                proc.wait()
                stderr = proc.stderr.read() if proc.stderr else ""  # type: ignore[union-attr]

                if (
                    proc.returncode != 0
                    and graph_format == "air"
                    and _fallback_retry_reason(stderr)
                ):
                    continue

                if proc.returncode != 0:
                    yield {
                        "event": "error",
                        "message": f"apxm exit {proc.returncode}: {stderr.strip()}",
                    }
                    return

                for node in self._graph.nodes:
                    yield {"event": "node_completed", "node_id": node.id, "name": node.name}

                yield {"event": "result", "data": None}
                return
            finally:
                os.unlink(tmp_path)

    def _build_subprocess_cmd(
        self,
        apxm_or_dekk: str,
        args: tuple[Any, ...],
        *,
        graph_format: str = "air",
    ) -> tuple[list[str], str]:
        graph_text = _build_graph_text(self._graph, graph_format)
        suffix = f".{graph_format}"
        with tempfile.NamedTemporaryFile(
            mode="w", suffix=suffix, delete=False, encoding="utf-8"
        ) as tmp:
            tmp.write(graph_text)
            tmp_path = tmp.name

        if Path(apxm_or_dekk).name == "dekk":
            cmd = [apxm_or_dekk, "apxm", "execute", tmp_path, "--json-output"]
        else:
            cmd = [apxm_or_dekk, "execute", tmp_path, "--json-output"]
        payload = _serialize_args(args)
        if payload is not None:
            cmd.extend(["--input", payload])
        return cmd, tmp_path

    def _fallback_subprocess(self, *args: Any) -> Any:
        apxm_bin = _find_apxm_binary()
        last_error: RuntimeError | None = None

        for graph_format in ("air", "json"):
            cmd, tmp_path = self._build_subprocess_cmd(
                apxm_bin, args, graph_format=graph_format
            )

            try:
                result = subprocess.run(
                    cmd, capture_output=True, text=True, check=False
                )

                if result.returncode != 0:
                    stderr = result.stderr.strip()
                    if graph_format == "air" and _fallback_retry_reason(stderr):
                        continue
                    raise RuntimeError(
                        f"apxm execute failed (exit {result.returncode}): {stderr}"
                    )

                stdout = result.stdout.strip()
                if not stdout:
                    return None
                try:
                    return json.loads(stdout)
                except json.JSONDecodeError:
                    return stdout
            except RuntimeError as exc:
                last_error = exc
            finally:
                os.unlink(tmp_path)

        if last_error is not None:
            raise last_error
        raise RuntimeError("apxm execute failed before producing output")
