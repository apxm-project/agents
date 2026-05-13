#!/usr/bin/env python3
"""Run repeated APXM graph benchmarks and emit a CSV summary.

This harness stays deliberately small:
- it shells out to `dekk apxm execute` for one-shot compile+run rows
- it supports `--precompile-artifacts` to compile once, then time repeated
  `dekk apxm run` artifact rows
- it supports `--compile-only` for backend-free validation
- it records wall-clock timing plus session-derived metrics when available

Usage:
  python3 examples/python/benchmarks/benchmark_e2e.py --compile-only
  python3 examples/python/benchmarks/benchmark_e2e.py --iterations 5
"""

from __future__ import annotations

import argparse
import csv
import json
import os
import shutil
import statistics
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
from dataclasses import asdict, dataclass
from datetime import datetime, timezone
try:
    from enum import StrEnum
except ImportError:  # Python 3.10 in some Dekk-managed test environments.
    from enum import Enum

    class StrEnum(str, Enum):
        pass
from pathlib import Path
from typing import Any


class BootstrapPath(StrEnum):
    CARGO_TOML = "Cargo.toml"
    CRATES = "crates"
    TOOLS = "tools"
    SCRIPTS = "scripts"


class BenchmarkPath(StrEnum):
    STRESS = "stress"
    DEMO_CODE_CRITIQUE = "demo_code_critique.py"
    DEFAULT_CSV = "demo_code_critique_benchmark.csv"
    SESSIONS = "sessions"


def _find_repo_root(start: Path) -> Path:
    for candidate in (start.resolve(), *start.resolve().parents):
        if (
            (candidate / BootstrapPath.CARGO_TOML.value).is_file()
            and (candidate / BootstrapPath.CRATES.value).is_dir()
        ):
            return candidate
    return Path.cwd().resolve()


REPO_ROOT = _find_repo_root(Path(__file__))
BENCHMARK_DIR = Path(__file__).resolve().parent
TOOLS_SCRIPT_DIR = REPO_ROOT / BootstrapPath.TOOLS.value / BootstrapPath.SCRIPTS.value
if str(TOOLS_SCRIPT_DIR) not in sys.path:
    sys.path.insert(0, str(TOOLS_SCRIPT_DIR))

from apxm_vllm_contract import (  # noqa: E402
    ApiRoute,
    DockerCommand,
    DockerFlag,
    DockerValue,
    EnvVar,
    HttpHeader,
    MediaType,
    RocmSmiFlag,
    ToolName,
    VllmDefaults,
    build_layout,
    enum_values,
    env_name,
    local_endpoint,
)

REPO_LAYOUT = build_layout(__file__)
REPO_ROOT = REPO_LAYOUT.repo_root
DEFAULT_GRAPH = BENCHMARK_DIR / BenchmarkPath.STRESS.value / BenchmarkPath.DEMO_CODE_CRITIQUE.value
DEFAULT_RESULTS_DIR = REPO_LAYOUT.benchmark_results_dir
DEFAULT_OUTPUT = DEFAULT_RESULTS_DIR / BenchmarkPath.DEFAULT_CSV.value
DEFAULT_SESSION_BASE = DEFAULT_RESULTS_DIR / BenchmarkPath.SESSIONS.value

FILE_MANIFEST = "manifest.json"
FILE_RESULTS = "results.json"
FILE_METRICS = "metrics.json"
FILE_NODE = "node.json"
FILE_NODE_STATUSES = "node_statuses.json"
DIR_COMPILER_DIAGNOSTICS = "compiler-diagnostics"
DIR_ARTIFACTS = "artifacts"
DIR_NODES = "nodes"
RUN_SUMMARY_PREFIX = "benchmark_e2e run"
PRECOMPILE_SUMMARY_PREFIX = "benchmark_e2e precompile"
PENDING_VALUE = "pending"
STATUS_SUCCEEDED = "succeeded"
STATUS_FAILED = "failed"
LIST_SEPARATOR = ";"
DEFAULT_VLLM = VllmDefaults()
DEFAULT_EVIDENCE_ENDPOINT = local_endpoint(host=DEFAULT_VLLM.host, port=DEFAULT_VLLM.port)
PASS_DSPY_OPTIMIZE = "dspy-optimize"
ATTR_BENCHMARK_MILESTONE = "benchmark_milestone"
EVIDENCE_SCHEMA_VERSION = 1


class CommandToken(StrEnum):
    DEKK = "dekk"
    APXM = "apxm"
    COMPILE = "compile"
    EXECUTE = "execute"
    RUN = "run"


class BenchmarkMode(StrEnum):
    COMPILE = "compile"
    EXECUTE = "execute"
    RUN_ARTIFACT = "run-artifact"


class OptimizationTarget(StrEnum):
    LATENCY = "latency"
    COST = "cost"
    TOKENS = "tokens"
    PARALLELISM = "parallelism"
    BALANCED = "balanced"


class CliFlag(StrEnum):
    EMIT_DIAGNOSTICS = "--emit-diagnostics"
    EMIT_SESSION = "--emit-session"
    OPT_LEVEL = "-O"
    OUTPUT = "-o"
    TARGET = "--target"
    TRACE = "--trace"


class CacheSaltScope(StrEnum):
    EXECUTION = "execution"


class MetricsKey(StrEnum):
    SCHEMA_VERSION = "schema_version"
    RUNTIME = "runtime"
    OBSERVED_GRAPH = "observed_graph"
    BACKENDS = "backends"
    GRAPHS = "graphs"
    TOKEN_ACCOUNTING = "token_accounting"
    COMPILER = "compiler"
    COMPILER_PASSES = "passes"
    COMPILER_SUMMARY = "summary"
    TOTAL = "total"
    INPUT_TOKENS = "input_tokens"
    OUTPUT_TOKENS = "output_tokens"
    TOTAL_TOKENS = "total_tokens"
    CACHED_INPUT_TOKENS = "cached_input_tokens"
    REASONING_OUTPUT_TOKENS = "reasoning_output_tokens"
    CALL_COUNT = "call_count"
    PINNED_BLOCKS = "pinned_blocks"
    PINNED_HANDLES = "pinned_handles"
    CRITICAL_PATH_LENGTH = "critical_path_length"


class DiagnosticsKey(StrEnum):
    PASS_METRICS = "pass_metrics"
    PASS_SUMMARY = "pass_summary"
    PASS_NAME = "pass_name"
    FIRED_COUNT = "fired_count"
    DURATION_MS = "duration_ms"
    TOTAL_PASSES = "total_passes"
    FIRED_PASSES = "fired_passes"
    ACTIVE_PASSES = "active_passes"
    TOTAL_OPS_ELIMINATED = "total_ops_eliminated"
    TOTAL_TOKENS_SAVED = "total_tokens_saved"


SCHEMA_VERSION_V2 = 2

KEY_DURATION_MS = "duration_ms"
KEY_FINAL_OUTPUT = "final_output"


def _backend_graphs(metrics: dict[str, Any]) -> list[dict[str, Any]]:
    backends = metrics.get(MetricsKey.BACKENDS, {})
    if not isinstance(backends, dict):
        return []

    graphs = backends.get(MetricsKey.GRAPHS, [])
    if not isinstance(graphs, list):
        return []
    return [graph for graph in graphs if isinstance(graph, dict)]

CSV_FIELDS = [
    "timestamp_utc",
    "graph",
    "mode",
    "variant",
    "backend_label",
    "opt_level",
    "optimization_target",
    "run_index",
    "trial_id",
    "run_order",
    "success",
    "wall_ms",
    "graph_duration_ms",
    "compile_wall_ms",
    "llm_call_count",
    "input_tokens",
    "output_tokens",
    "total_tokens",
    "cached_input_tokens",
    "reasoning_output_tokens",
    "pinned_blocks",
    "pinned_handles",
    "critical_path_length",
    "critical_milestones_json",
    "critical_milestone_last_ms",
    "critical_milestone_count",
    "observed_critical_path_ms",
    "observed_critical_path_finish_ms",
    "observed_critical_path_node_count",
    "observed_critical_path_nodes_json",
    "queue_wait_total_ms",
    "queue_wait_mean_ms",
    "queue_wait_p95_ms",
    "queue_wait_max_ms",
    "critical_queue_wait_total_ms",
    "critical_queue_wait_mean_ms",
    "diagnostics_path",
    "pass_count",
    "fired_passes",
    "active_passes",
    "total_ops_eliminated",
    "total_tokens_saved",
    "dspy_fired_count",
    "compiler_passes_ms",
    "artifact_path",
    "session_dir",
    "output_excerpt",
    "stderr_excerpt",
]


@dataclass
class SessionSummary:
    graph_duration_ms: int | None
    llm_call_count: int | None
    input_tokens: int | None
    output_tokens: int | None
    total_tokens: int | None
    cached_input_tokens: int | None
    reasoning_output_tokens: int | None
    pinned_blocks: int | None
    pinned_handles: int | None
    critical_path_length: int | None
    critical_milestones_json: str
    critical_milestone_last_ms: int | None
    critical_milestone_count: int
    observed_critical_path_ms: int | None
    observed_critical_path_finish_ms: int | None
    observed_critical_path_node_count: int | None
    observed_critical_path_nodes_json: str
    queue_wait_total_ms: int | None
    queue_wait_mean_ms: float | None
    queue_wait_p95_ms: int | None
    queue_wait_max_ms: int | None
    critical_queue_wait_total_ms: int | None
    critical_queue_wait_mean_ms: float | None
    final_output: str
    compiler_diagnostics: CompilerDiagnosticsSummary

    @classmethod
    def from_session(cls, session_root: Path) -> SessionSummary:
        graph_duration_ms: int | None = None
        final_output = ""
        llm_call_count: int | None = None
        input_tokens: int | None = None
        output_tokens: int | None = None
        total_tokens: int | None = None
        cached_input_tokens: int | None = None
        reasoning_output_tokens: int | None = None
        pinned_blocks: int | None = None
        pinned_handles: int | None = None
        critical_path_length: int | None = None
        critical_milestones_json = ""
        critical_milestone_last_ms: int | None = None
        critical_milestone_count = 0
        observed_critical_path_ms: int | None = None
        observed_critical_path_finish_ms: int | None = None
        observed_critical_path_node_count: int | None = None
        observed_critical_path_nodes_json = ""
        queue_wait_total_ms: int | None = None
        queue_wait_mean_ms: float | None = None
        queue_wait_p95_ms: int | None = None
        queue_wait_max_ms: int | None = None
        critical_queue_wait_total_ms: int | None = None
        critical_queue_wait_mean_ms: float | None = None
        compiler_diagnostics = CompilerDiagnosticsSummary.empty()

        manifest_path = session_root / FILE_MANIFEST
        if manifest_path.exists():
            manifest = json.loads(manifest_path.read_text())
            duration = manifest.get(KEY_DURATION_MS)
            if isinstance(duration, int):
                graph_duration_ms = duration

        results_path = session_root / FILE_RESULTS
        if results_path.exists():
            results = json.loads(results_path.read_text())
            out = results.get(KEY_FINAL_OUTPUT)
            if isinstance(out, str):
                final_output = out

        metrics_path = session_root / FILE_METRICS
        if metrics_path.exists():
            metrics = json.loads(metrics_path.read_text())
            version = metrics.get(MetricsKey.SCHEMA_VERSION)
            if version != SCHEMA_VERSION_V2:
                raise ValueError(
                    f"Expected metrics schema_version={SCHEMA_VERSION_V2}, got {version}"
                )

            total = (
                metrics.get(MetricsKey.RUNTIME, {})
                .get(MetricsKey.TOKEN_ACCOUNTING, {})
                .get(MetricsKey.TOTAL, {})
            )
            v = total.get(MetricsKey.CALL_COUNT)
            if isinstance(v, int):
                llm_call_count = v
            v = total.get(MetricsKey.INPUT_TOKENS)
            if isinstance(v, int):
                input_tokens = v
            v = total.get(MetricsKey.OUTPUT_TOKENS)
            if isinstance(v, int):
                output_tokens = v
            v = total.get(MetricsKey.TOTAL_TOKENS)
            if isinstance(v, int):
                total_tokens = v
            v = total.get(MetricsKey.CACHED_INPUT_TOKENS)
            if isinstance(v, int):
                cached_input_tokens = v
            v = total.get(MetricsKey.REASONING_OUTPUT_TOKENS)
            if isinstance(v, int):
                reasoning_output_tokens = v

            observed = (
                metrics.get(MetricsKey.RUNTIME, {})
                .get(MetricsKey.OBSERVED_GRAPH, {})
            )
            if isinstance(observed, dict):
                critical_path = observed.get("critical_path", {})
                if isinstance(critical_path, dict):
                    v = critical_path.get("duration_ms")
                    if isinstance(v, int):
                        observed_critical_path_ms = v
                    v = critical_path.get("finish_ms")
                    if isinstance(v, int):
                        observed_critical_path_finish_ms = v
                    v = critical_path.get("node_count")
                    if isinstance(v, int):
                        observed_critical_path_node_count = v
                    v = critical_path.get("nodes")
                    if isinstance(v, list):
                        observed_critical_path_nodes_json = json.dumps(
                            v,
                            separators=(",", ":"),
                        )
                queue_wait = observed.get("queue_wait", {})
                if isinstance(queue_wait, dict):
                    v = queue_wait.get("total_ms")
                    if isinstance(v, int):
                        queue_wait_total_ms = v
                    v = queue_wait.get("mean_ms")
                    if isinstance(v, int | float):
                        queue_wait_mean_ms = float(v)
                    v = queue_wait.get("p95_ms")
                    if isinstance(v, int):
                        queue_wait_p95_ms = v
                    v = queue_wait.get("max_ms")
                    if isinstance(v, int):
                        queue_wait_max_ms = v
                    v = queue_wait.get("critical_path_total_ms")
                    if isinstance(v, int):
                        critical_queue_wait_total_ms = v
                    v = queue_wait.get("critical_path_mean_ms")
                    if isinstance(v, int | float):
                        critical_queue_wait_mean_ms = float(v)

            vllm_graphs = _backend_graphs(metrics)
            if vllm_graphs:
                pb = sum(g.get(MetricsKey.PINNED_BLOCKS, 0) for g in vllm_graphs)
                ph = sum(g.get(MetricsKey.PINNED_HANDLES, 0) for g in vllm_graphs)
                cpl = sum(g.get(MetricsKey.CRITICAL_PATH_LENGTH, 0) for g in vllm_graphs)
                pinned_blocks = pb
                pinned_handles = ph
                critical_path_length = cpl
            compiler_diagnostics = _diagnostics_summary_from_payload(
                metrics,
                source=str(metrics_path),
            )

        critical_milestones = _critical_milestones_from_session(session_root)
        if critical_milestones:
            critical_milestones_json = json.dumps(
                critical_milestones,
                sort_keys=True,
                separators=(",", ":"),
            )
            critical_milestone_count = len(critical_milestones)
            finished_values = [
                milestone.get("finished_at_ms")
                for milestone in critical_milestones
                if isinstance(milestone.get("finished_at_ms"), int)
            ]
            if finished_values:
                critical_milestone_last_ms = max(finished_values)

        return cls(
            graph_duration_ms=graph_duration_ms,
            llm_call_count=llm_call_count,
            input_tokens=input_tokens,
            output_tokens=output_tokens,
            total_tokens=total_tokens,
            cached_input_tokens=cached_input_tokens,
            reasoning_output_tokens=reasoning_output_tokens,
            pinned_blocks=pinned_blocks,
            pinned_handles=pinned_handles,
            critical_path_length=critical_path_length,
            critical_milestones_json=critical_milestones_json,
            critical_milestone_last_ms=critical_milestone_last_ms,
            critical_milestone_count=critical_milestone_count,
            observed_critical_path_ms=observed_critical_path_ms,
            observed_critical_path_finish_ms=observed_critical_path_finish_ms,
            observed_critical_path_node_count=observed_critical_path_node_count,
            observed_critical_path_nodes_json=observed_critical_path_nodes_json,
            queue_wait_total_ms=queue_wait_total_ms,
            queue_wait_mean_ms=queue_wait_mean_ms,
            queue_wait_p95_ms=queue_wait_p95_ms,
            queue_wait_max_ms=queue_wait_max_ms,
            critical_queue_wait_total_ms=critical_queue_wait_total_ms,
            critical_queue_wait_mean_ms=critical_queue_wait_mean_ms,
            final_output=final_output,
            compiler_diagnostics=compiler_diagnostics,
        )


@dataclass
class CompilerDiagnosticsSummary:
    diagnostics_path: str
    pass_count: int | None
    fired_passes: str
    active_passes: str
    total_ops_eliminated: int | None
    total_tokens_saved: int | None
    dspy_fired_count: int | None
    compiler_passes_ms: float | None

    @classmethod
    def empty(cls) -> CompilerDiagnosticsSummary:
        return cls(
            diagnostics_path="",
            pass_count=None,
            fired_passes="",
            active_passes="",
            total_ops_eliminated=None,
            total_tokens_saved=None,
            dspy_fired_count=None,
            compiler_passes_ms=None,
        )

    def as_fields(self) -> dict[str, Any]:
        return {
            "diagnostics_path": self.diagnostics_path,
            "pass_count": "" if self.pass_count is None else self.pass_count,
            "fired_passes": self.fired_passes,
            "active_passes": self.active_passes,
            "total_ops_eliminated": ""
            if self.total_ops_eliminated is None
            else self.total_ops_eliminated,
            "total_tokens_saved": ""
            if self.total_tokens_saved is None
            else self.total_tokens_saved,
            "dspy_fired_count": ""
            if self.dspy_fired_count is None
            else self.dspy_fired_count,
            "compiler_passes_ms": ""
            if self.compiler_passes_ms is None
            else f"{self.compiler_passes_ms:.3f}",
        }


@dataclass
class RunRecord:
    timestamp_utc: str
    graph: str
    mode: str
    variant: str
    backend_label: str
    opt_level: int
    optimization_target: str
    run_index: int
    trial_id: int
    run_order: int
    success: bool
    wall_ms: float
    graph_duration_ms: int | None
    compile_wall_ms: float | None
    llm_call_count: int | None
    input_tokens: int | None
    output_tokens: int | None
    total_tokens: int | None
    cached_input_tokens: int | None
    reasoning_output_tokens: int | None
    pinned_blocks: int | None
    pinned_handles: int | None
    critical_path_length: int | None
    critical_milestones_json: str
    critical_milestone_last_ms: int | None
    critical_milestone_count: int
    observed_critical_path_ms: int | None
    observed_critical_path_finish_ms: int | None
    observed_critical_path_node_count: int | None
    observed_critical_path_nodes_json: str
    queue_wait_total_ms: int | None
    queue_wait_mean_ms: float | None
    queue_wait_p95_ms: int | None
    queue_wait_max_ms: int | None
    critical_queue_wait_total_ms: int | None
    critical_queue_wait_mean_ms: float | None
    compiler_diagnostics: CompilerDiagnosticsSummary
    artifact_path: str
    session_dir: str
    output_excerpt: str
    stderr_excerpt: str

    def as_row(self) -> dict[str, Any]:
        return {
            "timestamp_utc": self.timestamp_utc,
            "graph": self.graph,
            "mode": self.mode,
            "variant": self.variant,
            "backend_label": self.backend_label,
            "opt_level": self.opt_level,
            "optimization_target": self.optimization_target,
            "run_index": self.run_index,
            "trial_id": self.trial_id,
            "run_order": self.run_order,
            "success": "true" if self.success else "false",
            "wall_ms": f"{self.wall_ms:.3f}",
            "graph_duration_ms": "" if self.graph_duration_ms is None else self.graph_duration_ms,
            "compile_wall_ms": ""
            if self.compile_wall_ms is None
            else f"{self.compile_wall_ms:.3f}",
            "llm_call_count": "" if self.llm_call_count is None else self.llm_call_count,
            "input_tokens": "" if self.input_tokens is None else self.input_tokens,
            "output_tokens": "" if self.output_tokens is None else self.output_tokens,
            "total_tokens": "" if self.total_tokens is None else self.total_tokens,
            "cached_input_tokens": ""
            if self.cached_input_tokens is None
            else self.cached_input_tokens,
            "reasoning_output_tokens": ""
            if self.reasoning_output_tokens is None
            else self.reasoning_output_tokens,
            "pinned_blocks": "" if self.pinned_blocks is None else self.pinned_blocks,
            "pinned_handles": "" if self.pinned_handles is None else self.pinned_handles,
            "critical_path_length": "" if self.critical_path_length is None else self.critical_path_length,
            "critical_milestones_json": self.critical_milestones_json,
            "critical_milestone_last_ms": ""
            if self.critical_milestone_last_ms is None
            else self.critical_milestone_last_ms,
            "critical_milestone_count": self.critical_milestone_count,
            "observed_critical_path_ms": ""
            if self.observed_critical_path_ms is None
            else self.observed_critical_path_ms,
            "observed_critical_path_finish_ms": ""
            if self.observed_critical_path_finish_ms is None
            else self.observed_critical_path_finish_ms,
            "observed_critical_path_node_count": ""
            if self.observed_critical_path_node_count is None
            else self.observed_critical_path_node_count,
            "observed_critical_path_nodes_json": self.observed_critical_path_nodes_json,
            "queue_wait_total_ms": ""
            if self.queue_wait_total_ms is None
            else self.queue_wait_total_ms,
            "queue_wait_mean_ms": ""
            if self.queue_wait_mean_ms is None
            else f"{self.queue_wait_mean_ms:.3f}",
            "queue_wait_p95_ms": ""
            if self.queue_wait_p95_ms is None
            else self.queue_wait_p95_ms,
            "queue_wait_max_ms": ""
            if self.queue_wait_max_ms is None
            else self.queue_wait_max_ms,
            "critical_queue_wait_total_ms": ""
            if self.critical_queue_wait_total_ms is None
            else self.critical_queue_wait_total_ms,
            "critical_queue_wait_mean_ms": ""
            if self.critical_queue_wait_mean_ms is None
            else f"{self.critical_queue_wait_mean_ms:.3f}",
            **self.compiler_diagnostics.as_fields(),
            "artifact_path": self.artifact_path,
            "session_dir": self.session_dir,
            "output_excerpt": self.output_excerpt,
            "stderr_excerpt": self.stderr_excerpt,
        }


@dataclass(frozen=True)
class ArtifactBuild:
    opt_level: int
    artifact_path: Path
    diagnostics_path: Path | None
    compiler_diagnostics: CompilerDiagnosticsSummary
    wall_ms: float
    success: bool
    stderr_excerpt: str


@dataclass(frozen=True)
class BenchmarkEvidence:
    graph: str
    output_csv: str
    session_base: str
    backend_label: str
    variant: str
    opt_levels: list[int]
    iterations: int
    precompile_artifacts: bool
    emit_compiler_diagnostics: bool
    success: bool
    run_count: int


@dataclass(frozen=True)
class RepoEvidence:
    path: str
    commit: str | None
    dirty: bool
    branch: str | None = None
    origin_apxm: str | None = None


@dataclass(frozen=True)
class ModelEvidence:
    model_ref: str
    served_model_id: str
    backend_name: str
    hf_home_host: str
    port: str


@dataclass(frozen=True)
class SlurmEvidence:
    job_id: str
    job_nodelist: str
    job_partition: str


@dataclass(frozen=True)
class ContainerEvidence:
    image: str
    inspect: Any


@dataclass(frozen=True)
class EndpointEvidence:
    base_url: str
    models: Any
    scheduler: Any


@dataclass(frozen=True)
class HostEvidence:
    hostname: Any
    gpu_smi: Any


@dataclass(frozen=True)
class EvidenceManifest:
    schema_version: int
    created_at_utc: str
    benchmark: BenchmarkEvidence
    apxm: RepoEvidence
    vllm_fork: RepoEvidence
    model: ModelEvidence
    slurm: SlurmEvidence
    container: ContainerEvidence
    endpoint: EndpointEvidence
    host: HostEvidence


def _parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--graph", type=Path, default=DEFAULT_GRAPH, help="Graph source to run")
    parser.add_argument(
        "--opt-level",
        dest="opt_levels",
        type=int,
        action="append",
        help="Optimization level to benchmark (repeatable, default: 0 and 2)",
    )
    parser.add_argument(
        "--target",
        default=OptimizationTarget.BALANCED.value,
        choices=[target.value for target in OptimizationTarget],
        help="Optimization target passed to `dekk apxm compile/execute`.",
    )
    parser.add_argument(
        "--iterations",
        type=int,
        default=3,
        help="Number of runs per optimization level",
    )
    parser.add_argument(
        "--compile-only",
        action="store_true",
        help="Run `dekk apxm compile` instead of `dekk apxm execute`",
    )
    parser.add_argument(
        "--precompile-artifacts",
        action="store_true",
        help=(
            "Compile one artifact per opt level, then benchmark repeated "
            "`dekk apxm run` rows so runtime samples are not dominated by compile time."
        ),
    )
    parser.add_argument(
        "--artifact-dir",
        type=Path,
        default=None,
        help="Directory for artifacts created by --precompile-artifacts.",
    )
    parser.add_argument(
        "--emit-compiler-diagnostics",
        action="store_true",
        help=(
            "Ask compile steps for per-pass diagnostics in compile-only or "
            "precompiled-artifact mode."
        ),
    )
    parser.add_argument(
        "--diagnostics-dir",
        type=Path,
        default=None,
        help="Directory for compile-only diagnostics JSON files.",
    )
    parser.add_argument(
        "--output",
        type=Path,
        default=DEFAULT_OUTPUT,
        help="CSV output path",
    )
    parser.add_argument(
        "--session-base",
        type=Path,
        default=DEFAULT_SESSION_BASE,
        help="Base directory for emitted sessions in execute mode",
    )
    parser.add_argument(
        "--backend-label",
        default="configured",
        help="Reporting label for the configured backend",
    )
    parser.add_argument(
        "--variant",
        default="default",
        help="Reporting label for this compiler/runtime configuration.",
    )
    parser.add_argument(
        "--apxm-config",
        type=Path,
        default=None,
        help="APXM config path exported to Dekk and Python graph emission",
    )
    parser.add_argument(
        "--trace",
        default=None,
        help="Optional APXM trace level for execute mode",
    )
    parser.add_argument(
        "--runtime-arg",
        dest="runtime_args",
        action="append",
        default=[],
        help=(
            "Runtime argument passed to the entry flow. Repeat for multiple "
            "parameters. Used by both execute and precompiled artifact runs."
        ),
    )
    parser.add_argument(
        "--append",
        action="store_true",
        help="Append to an existing CSV instead of overwriting it",
    )
    parser.add_argument(
        "--interleave-opt-levels",
        action="store_true",
        help="Run trial 1 for every opt level before trial 2 to reduce warm-cache bias.",
    )
    parser.add_argument(
        "--evidence-manifest",
        type=Path,
        default=None,
        help="Write a JSON sidecar with system, backend, and run evidence.",
    )
    parser.add_argument(
        "--evidence-endpoint",
        default=DEFAULT_EVIDENCE_ENDPOINT,
        help="OpenAI-compatible endpoint used for evidence probes.",
    )
    return parser.parse_args()


def _collapse_text(text: str, limit: int = 240) -> str:
    collapsed = " ".join(text.split())
    if len(collapsed) <= limit:
        return collapsed
    return collapsed[: limit - 3] + "..."


def _require_dekk() -> None:
    if shutil.which(CommandToken.DEKK.value) is None:
        raise SystemExit("error: `dekk` is not on PATH")


def _resolve_session_root(session_base: Path) -> Path:
    direct = session_base / FILE_RESULTS
    if direct.exists():
        return session_base

    children = [child for child in session_base.iterdir() if child.is_dir()]
    completed_children = [
        child for child in children if (child / FILE_RESULTS).is_file()
    ]
    if len(completed_children) == 1:
        return completed_children[0]
    if len(completed_children) > 1:
        return max(
            completed_children,
            key=lambda child: (child / FILE_RESULTS).stat().st_mtime,
        )

    raise FileNotFoundError(
        f"no completed session with {FILE_RESULTS} under {session_base}; "
        f"found {len(children)} child session "
        f"{'directory' if len(children) == 1 else 'directories'}"
    )


def _read_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text())


def _load_node_metadata(session_root: Path) -> dict[int, dict[str, Any]]:
    nodes_dir = session_root / DIR_NODES
    if not nodes_dir.is_dir():
        return {}

    metadata: dict[int, dict[str, Any]] = {}
    for node_path in sorted(nodes_dir.glob(f"*/{FILE_NODE}")):
        try:
            node_payload = _read_json(node_path)
        except (OSError, json.JSONDecodeError):
            continue
        node_id = node_payload.get("id")
        if isinstance(node_id, int):
            metadata[node_id] = node_payload
    return metadata


def _critical_milestones_from_session(session_root: Path) -> list[dict[str, Any]]:
    statuses_path = session_root / FILE_NODE_STATUSES
    if not statuses_path.is_file():
        return []

    try:
        statuses = json.loads(statuses_path.read_text())
    except (OSError, json.JSONDecodeError):
        return []
    if not isinstance(statuses, list):
        return []

    node_metadata = _load_node_metadata(session_root)
    milestones: list[dict[str, Any]] = []
    for status in statuses:
        if not isinstance(status, dict):
            continue
        node_id = status.get("node_id")
        if not isinstance(node_id, int):
            continue
        node_payload = node_metadata.get(node_id, {})
        attrs = node_payload.get("attributes")
        if not isinstance(attrs, dict):
            continue
        marker = attrs.get(ATTR_BENCHMARK_MILESTONE)
        if not isinstance(marker, str) or not marker:
            continue
        if "critical" not in marker.lower():
            continue

        milestone = {
            "milestone": marker,
            "node_id": node_id,
            "node_name": node_payload.get("name"),
            "op": node_payload.get("op"),
            "priority": attrs.get("priority"),
            "started_at_ms": status.get("started_at_ms"),
            "finished_at_ms": status.get("finished_at_ms"),
            "duration_ms": status.get("duration_ms"),
        }
        milestones.append(
            {key: value for key, value in milestone.items() if value is not None}
        )

    milestones.sort(
        key=lambda item: (
            item.get("finished_at_ms")
            if isinstance(item.get("finished_at_ms"), int)
            else -1,
            item.get("node_id") if isinstance(item.get("node_id"), int) else -1,
        )
    )
    return milestones


def _string_list(value: Any) -> str:
    if not isinstance(value, list):
        return ""
    return LIST_SEPARATOR.join(str(item) for item in value)


def _diagnostics_summary_from_payload(
    data: dict[str, Any],
    *,
    source: str = "",
) -> CompilerDiagnosticsSummary:
    compiler = data.get(MetricsKey.COMPILER)
    if isinstance(compiler, dict):
        data = compiler

    pass_metrics = data.get(DiagnosticsKey.PASS_METRICS)
    if pass_metrics is None:
        pass_metrics = data.get(MetricsKey.COMPILER_PASSES)
    if not isinstance(pass_metrics, list):
        pass_metrics = []

    pass_summary = data.get(DiagnosticsKey.PASS_SUMMARY)
    if pass_summary is None:
        pass_summary = data.get(MetricsKey.COMPILER_SUMMARY)
    if not isinstance(pass_summary, dict):
        pass_summary = {}

    dspy_fired_count: int | None = None
    for metric in pass_metrics:
        if not isinstance(metric, dict):
            continue
        if metric.get(DiagnosticsKey.PASS_NAME) != PASS_DSPY_OPTIMIZE:
            continue
        fired_count = metric.get(DiagnosticsKey.FIRED_COUNT)
        if isinstance(fired_count, int):
            dspy_fired_count = (dspy_fired_count or 0) + fired_count

    compiler_passes_ms = 0.0
    saw_duration = False
    for metric in pass_metrics:
        if not isinstance(metric, dict):
            continue
        duration = metric.get(DiagnosticsKey.DURATION_MS)
        if isinstance(duration, int | float):
            compiler_passes_ms += float(duration)
            saw_duration = True

    def _int_summary(key: DiagnosticsKey) -> int | None:
        value = pass_summary.get(key)
        return value if isinstance(value, int) else None

    if not pass_metrics and not pass_summary:
        return CompilerDiagnosticsSummary.empty()

    return CompilerDiagnosticsSummary(
        diagnostics_path=source,
        pass_count=_int_summary(DiagnosticsKey.TOTAL_PASSES) or len(pass_metrics),
        fired_passes=_string_list(pass_summary.get(DiagnosticsKey.FIRED_PASSES)),
        active_passes=_string_list(pass_summary.get(DiagnosticsKey.ACTIVE_PASSES)),
        total_ops_eliminated=_int_summary(DiagnosticsKey.TOTAL_OPS_ELIMINATED),
        total_tokens_saved=_int_summary(DiagnosticsKey.TOTAL_TOKENS_SAVED),
        dspy_fired_count=dspy_fired_count,
        compiler_passes_ms=compiler_passes_ms if saw_duration else None,
    )


def _diagnostics_summary(path: Path | None) -> CompilerDiagnosticsSummary:
    if path is None or not path.is_file():
        return CompilerDiagnosticsSummary.empty()

    return _diagnostics_summary_from_payload(_read_json(path), source=str(path))


def _env_value(env_var: EnvVar) -> str:
    return os.environ.get(env_name(env_var), "")


def _endpoint_url(base_url: str, route: ApiRoute) -> str:
    return f"{base_url.rstrip('/')}/{route.value}"


def _command_prefix(apxm_config: Path | None) -> list[str]:
    return enum_values([CommandToken.DEKK, CommandToken.APXM])


def _command_env(apxm_config: Path | None) -> dict[str, str]:
    """Build the per-iteration child env.

    Always sets ``APXM_VLLM_CACHE_SALT=execution`` so each benchmark
    iteration salts the vLLM prefix cache by execution_id. This isolates
    back-to-back O0/O2 sweeps from each other's KV state. Production runs
    (``dekk apxm execute`` invoked directly) do not pass through this helper,
    so the prefix cache is reused across executions as designed.
    """
    env = os.environ.copy()
    if apxm_config is not None:
        env[env_name(EnvVar.APXM_CONFIG)] = str(apxm_config)
    env[env_name(EnvVar.APXM_VLLM_CACHE_SALT)] = CacheSaltScope.EXECUTION.value
    return env


def _compile_to_artifact(
    graph: Path,
    opt_level: int,
    target: str,
    artifact_path: Path,
    apxm_config: Path | None,
    diagnostics_path: Path | None,
) -> tuple[subprocess.CompletedProcess[str], float]:
    artifact_path.parent.mkdir(parents=True, exist_ok=True)
    diagnostics_args: list[str] = []
    if diagnostics_path is not None:
        diagnostics_path.parent.mkdir(parents=True, exist_ok=True)
        diagnostics_args = enum_values([CliFlag.EMIT_DIAGNOSTICS, str(diagnostics_path)])
    if artifact_path.exists():
        artifact_path.unlink()
    cmd = enum_values([
        *_command_prefix(apxm_config),
        CommandToken.COMPILE,
        str(graph),
        CliFlag.OUTPUT,
        str(artifact_path),
        CliFlag.OPT_LEVEL,
        str(opt_level),
        CliFlag.TARGET,
        target,
        *diagnostics_args,
    ])
    start = time.perf_counter()
    result = subprocess.run(
        cmd,
        capture_output=True,
        text=True,
        cwd=REPO_ROOT,
        env=_command_env(apxm_config),
    )
    wall_ms = (time.perf_counter() - start) * 1000.0
    return result, wall_ms


def _run_compile(
    graph: Path,
    opt_level: int,
    target: str,
    apxm_config: Path | None,
    diagnostics_path: Path | None,
) -> tuple[subprocess.CompletedProcess[str], float]:
    with tempfile.TemporaryDirectory(prefix="apxm-bench-compile-") as tmp_dir:
        artifact_path = Path(tmp_dir) / f"{graph.stem}-O{opt_level}.apxmobj"
        return _compile_to_artifact(
            graph,
            opt_level,
            target,
            artifact_path,
            apxm_config,
            diagnostics_path,
        )


def _run_execute(
    graph: Path,
    opt_level: int,
    target: str,
    session_base: Path,
    trace: str | None,
    apxm_config: Path | None,
    runtime_args: list[str],
) -> tuple[subprocess.CompletedProcess[str], float]:
    session_base.mkdir(parents=True, exist_ok=True)
    cmd = enum_values([
        *_command_prefix(apxm_config),
        CommandToken.EXECUTE,
        CliFlag.OPT_LEVEL,
        str(opt_level),
        CliFlag.TARGET,
        target,
    ])
    if trace:
        cmd.extend(enum_values([CliFlag.TRACE, trace]))
    cmd.extend(enum_values([CliFlag.EMIT_SESSION, str(session_base), str(graph), *runtime_args]))

    start = time.perf_counter()
    result = subprocess.run(
        cmd,
        capture_output=True,
        text=True,
        cwd=REPO_ROOT,
        env=_command_env(apxm_config),
    )
    wall_ms = (time.perf_counter() - start) * 1000.0
    return result, wall_ms


def _run_artifact(
    artifact_path: Path,
    target: str,
    session_base: Path,
    trace: str | None,
    apxm_config: Path | None,
    runtime_args: list[str],
) -> tuple[subprocess.CompletedProcess[str], float]:
    session_base.mkdir(parents=True, exist_ok=True)
    cmd = enum_values([
        *_command_prefix(apxm_config),
        CommandToken.RUN,
        CliFlag.TARGET,
        target,
    ])
    if trace:
        cmd.extend(enum_values([CliFlag.TRACE, trace]))
    cmd.extend(
        enum_values([CliFlag.EMIT_SESSION, str(session_base), str(artifact_path), *runtime_args])
    )

    start = time.perf_counter()
    result = subprocess.run(
        cmd,
        capture_output=True,
        text=True,
        cwd=REPO_ROOT,
        env=_command_env(apxm_config),
    )
    wall_ms = (time.perf_counter() - start) * 1000.0
    return result, wall_ms


def _run_once(
    graph: Path,
    opt_level: int,
    target: str,
    run_index: int,
    trial_id: int,
    run_order: int,
    compile_only: bool,
    variant: str,
    backend_label: str,
    trace: str | None,
    session_parent: Path,
    apxm_config: Path | None,
    emit_compiler_diagnostics: bool,
    diagnostics_parent: Path,
    artifact_build: ArtifactBuild | None,
    runtime_args: list[str],
) -> RunRecord:
    timestamp = datetime.now(timezone.utc).isoformat()
    mode = (
        BenchmarkMode.COMPILE.value
        if compile_only
        else BenchmarkMode.RUN_ARTIFACT.value
        if artifact_build
        else BenchmarkMode.EXECUTE.value
    )

    if compile_only:
        diagnostics_path = (
            diagnostics_parent
            / graph.stem
            / f"O{opt_level}-run-{run_index}-order-{run_order}.json"
            if emit_compiler_diagnostics
            else None
        )
        result, wall_ms = _run_compile(
            graph,
            opt_level,
            target,
            apxm_config,
            diagnostics_path,
        )
        compiler_diagnostics = (
            _diagnostics_summary(diagnostics_path)
            if result.returncode == 0
            else CompilerDiagnosticsSummary.empty()
        )
        return RunRecord(
            timestamp_utc=timestamp,
            graph=str(graph),
            mode=mode,
            variant=variant,
            backend_label=backend_label,
            opt_level=opt_level,
            optimization_target=target,
            run_index=run_index,
            trial_id=trial_id,
            run_order=run_order,
            success=result.returncode == 0,
            wall_ms=wall_ms,
            graph_duration_ms=None,
            compile_wall_ms=wall_ms,
            llm_call_count=None,
            input_tokens=None,
            output_tokens=None,
            total_tokens=None,
            cached_input_tokens=None,
            reasoning_output_tokens=None,
            pinned_blocks=None,
            pinned_handles=None,
            critical_path_length=None,
            critical_milestones_json="",
            critical_milestone_last_ms=None,
            critical_milestone_count=0,
            observed_critical_path_ms=None,
            observed_critical_path_finish_ms=None,
            observed_critical_path_node_count=None,
            observed_critical_path_nodes_json="",
            queue_wait_total_ms=None,
            queue_wait_mean_ms=None,
            queue_wait_p95_ms=None,
            queue_wait_max_ms=None,
            critical_queue_wait_total_ms=None,
            critical_queue_wait_mean_ms=None,
            compiler_diagnostics=compiler_diagnostics,
            artifact_path="",
            session_dir="",
            output_excerpt="",
            stderr_excerpt=_collapse_text(result.stderr or result.stdout),
        )

    session_base = session_parent / f"opt-{opt_level}" / f"run-{run_index}"
    if artifact_build is not None:
        result, wall_ms = _run_artifact(
            artifact_build.artifact_path,
            target,
            session_base,
            trace,
            apxm_config,
            runtime_args,
        )
    else:
        result, wall_ms = _run_execute(
            graph, opt_level, target, session_base, trace, apxm_config, runtime_args
        )
    session_root: Path | None = None
    summary: SessionSummary | None = None
    if result.returncode == 0:
        session_root = _resolve_session_root(session_base)
        summary = SessionSummary.from_session(session_root)
    compiler_diagnostics = CompilerDiagnosticsSummary.empty()
    if artifact_build is not None:
        compiler_diagnostics = artifact_build.compiler_diagnostics
    elif summary is not None:
        compiler_diagnostics = summary.compiler_diagnostics
    return RunRecord(
        timestamp_utc=timestamp,
        graph=str(graph),
        mode=mode,
        variant=variant,
        backend_label=backend_label,
        opt_level=opt_level,
        optimization_target=target,
        run_index=run_index,
        trial_id=trial_id,
        run_order=run_order,
        success=result.returncode == 0,
        wall_ms=wall_ms,
        graph_duration_ms=summary.graph_duration_ms if summary else None,
        compile_wall_ms=artifact_build.wall_ms if artifact_build is not None else None,
        llm_call_count=summary.llm_call_count if summary else None,
        input_tokens=summary.input_tokens if summary else None,
        output_tokens=summary.output_tokens if summary else None,
        total_tokens=summary.total_tokens if summary else None,
        cached_input_tokens=summary.cached_input_tokens if summary else None,
        reasoning_output_tokens=summary.reasoning_output_tokens if summary else None,
        pinned_blocks=summary.pinned_blocks if summary else None,
        pinned_handles=summary.pinned_handles if summary else None,
        critical_path_length=summary.critical_path_length if summary else None,
        critical_milestones_json=summary.critical_milestones_json if summary else "",
        critical_milestone_last_ms=summary.critical_milestone_last_ms if summary else None,
        critical_milestone_count=summary.critical_milestone_count if summary else 0,
        observed_critical_path_ms=summary.observed_critical_path_ms if summary else None,
        observed_critical_path_finish_ms=summary.observed_critical_path_finish_ms if summary else None,
        observed_critical_path_node_count=summary.observed_critical_path_node_count if summary else None,
        observed_critical_path_nodes_json=summary.observed_critical_path_nodes_json if summary else "",
        queue_wait_total_ms=summary.queue_wait_total_ms if summary else None,
        queue_wait_mean_ms=summary.queue_wait_mean_ms if summary else None,
        queue_wait_p95_ms=summary.queue_wait_p95_ms if summary else None,
        queue_wait_max_ms=summary.queue_wait_max_ms if summary else None,
        critical_queue_wait_total_ms=summary.critical_queue_wait_total_ms if summary else None,
        critical_queue_wait_mean_ms=summary.critical_queue_wait_mean_ms if summary else None,
        compiler_diagnostics=compiler_diagnostics,
        artifact_path=str(artifact_build.artifact_path) if artifact_build is not None else "",
        session_dir=str(session_root) if session_root is not None else "",
        output_excerpt=_collapse_text(summary.final_output if summary else ""),
        stderr_excerpt=_collapse_text(result.stderr or result.stdout),
    )


def _artifact_name(graph: Path, opt_level: int, target: str) -> str:
    safe_target = "".join(ch if ch.isalnum() or ch in ("-", "_") else "_" for ch in target)
    return f"{graph.stem}-O{opt_level}-{safe_target}.apxmobj"


def _precompile_artifacts(
    graph: Path,
    opt_levels: list[int],
    target: str,
    artifact_dir: Path,
    diagnostics_parent: Path,
    apxm_config: Path | None,
    emit_compiler_diagnostics: bool,
) -> dict[int, ArtifactBuild]:
    builds: dict[int, ArtifactBuild] = {}
    manifest: list[dict[str, Any]] = []

    for opt_level in opt_levels:
        artifact_path = artifact_dir / _artifact_name(graph, opt_level, target)
        diagnostics_path = (
            diagnostics_parent / graph.stem / f"O{opt_level}-precompile.json"
            if emit_compiler_diagnostics
            else None
        )
        result, wall_ms = _compile_to_artifact(
            graph,
            opt_level,
            target,
            artifact_path,
            apxm_config,
            diagnostics_path,
        )
        compiler_diagnostics = (
            _diagnostics_summary(diagnostics_path)
            if result.returncode == 0
            else CompilerDiagnosticsSummary.empty()
        )
        build = ArtifactBuild(
            opt_level=opt_level,
            artifact_path=artifact_path,
            diagnostics_path=diagnostics_path,
            compiler_diagnostics=compiler_diagnostics,
            wall_ms=wall_ms,
            success=result.returncode == 0,
            stderr_excerpt=_collapse_text(result.stderr or result.stdout),
        )
        builds[opt_level] = build
        _print_precompile_summary(build, target)
        manifest.append(
            {
                "opt_level": opt_level,
                "optimization_target": target,
                "success": build.success,
                "wall_ms": round(build.wall_ms, 3),
                "artifact": str(build.artifact_path),
                "diagnostics": str(build.diagnostics_path)
                if build.diagnostics_path is not None
                else None,
                "pass_count": build.compiler_diagnostics.pass_count,
                "fired_passes": build.compiler_diagnostics.fired_passes,
                "dspy_fired_count": build.compiler_diagnostics.dspy_fired_count,
                "stderr_excerpt": build.stderr_excerpt,
            }
        )
        if not build.success:
            break

    artifact_dir.mkdir(parents=True, exist_ok=True)
    (artifact_dir / "precompile-manifest.json").write_text(
        json.dumps(manifest, indent=2) + "\n"
    )
    failures = [build for build in builds.values() if not build.success]
    if failures:
        failed = failures[0]
        raise SystemExit(
            f"error: artifact precompile failed for O{failed.opt_level}: "
            f"{failed.stderr_excerpt or 'no compiler output'}"
        )
    return builds


def _write_csv(records: list[RunRecord], output: Path, append: bool) -> None:
    output.parent.mkdir(parents=True, exist_ok=True)
    mode = "a" if append and output.exists() else "w"
    write_header = mode == "w"
    if mode == "a":
        with output.open(newline="") as existing:
            reader = csv.DictReader(existing)
            existing_fields = reader.fieldnames or []
            if existing_fields != CSV_FIELDS:
                old_rows = list(reader)
                with output.open("w", newline="") as rewritten:
                    writer = csv.DictWriter(rewritten, fieldnames=CSV_FIELDS)
                    writer.writeheader()
                    for row in old_rows:
                        writer.writerow({field: row.get(field, "") for field in CSV_FIELDS})
                write_header = False
    with output.open(mode, newline="") as handle:
        writer = csv.DictWriter(handle, fieldnames=CSV_FIELDS)
        if write_header:
            writer.writeheader()
        for record in records:
            writer.writerow(record.as_row())


def _git(repo: Path, *args: str) -> str | None:
    result = subprocess.run(
        [ToolName.GIT.value, "-C", str(repo), *args],
        capture_output=True,
        text=True,
        cwd=REPO_ROOT,
    )
    if result.returncode != 0:
        return None
    value = result.stdout.strip()
    return value if value else None


def _command_json(cmd: list[str], *, timeout: float = 30.0) -> Any:
    try:
        result = subprocess.run(
            cmd,
            capture_output=True,
            text=True,
            cwd=REPO_ROOT,
            timeout=timeout,
        )
    except (OSError, subprocess.TimeoutExpired) as exc:
        return {"error": repr(exc)}
    if result.returncode != 0:
        return {
            "error": result.stderr.strip() or result.stdout.strip(),
            "returncode": result.returncode,
        }
    text = result.stdout.strip()
    try:
        return json.loads(text) if text else None
    except json.JSONDecodeError:
        return text


def _http_json(url: str, *, timeout: float = 15.0) -> Any:
    request = urllib.request.Request(
        url,
        headers={HttpHeader.CONTENT_TYPE.value: MediaType.JSON.value},
        method="GET",
    )
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            raw = response.read().decode("utf-8")
    except (urllib.error.URLError, TimeoutError) as exc:
        return {"error": repr(exc)}
    try:
        return json.loads(raw) if raw else None
    except json.JSONDecodeError as exc:
        return {"error": repr(exc), "raw": raw[:500]}


def _image_inspect(image: str | None) -> Any:
    if not image:
        return None
    return _command_json(
        enum_values(
            [
                ToolName.DOCKER,
                DockerCommand.IMAGE,
                DockerCommand.INSPECT,
                image,
                DockerFlag.FORMAT,
                DockerValue.JSON_OBJECT_FORMAT,
            ]
        )
    )


def _write_evidence_manifest(
    *,
    args: argparse.Namespace,
    records: list[RunRecord],
    output: Path,
) -> Path:
    endpoint = args.evidence_endpoint.rstrip("/")
    vllm_dir = REPO_LAYOUT.vllm_dir
    manifest_path = (
        args.evidence_manifest.resolve()
        if args.evidence_manifest is not None
        else output.with_suffix(".evidence.json")
    )
    image = _env_value(EnvVar.APXM_VLLM_IMAGE)
    manifest = EvidenceManifest(
        schema_version=EVIDENCE_SCHEMA_VERSION,
        created_at_utc=datetime.now(timezone.utc).isoformat(),
        benchmark=BenchmarkEvidence(
            graph=str(args.graph.resolve()),
            output_csv=str(output),
            session_base=str(args.session_base.resolve()),
            backend_label=args.backend_label,
            variant=args.variant,
            opt_levels=args.opt_levels or [0, 2],
            iterations=args.iterations,
            precompile_artifacts=args.precompile_artifacts,
            emit_compiler_diagnostics=args.emit_compiler_diagnostics,
            success=all(record.success for record in records),
            run_count=len(records),
        ),
        apxm=RepoEvidence(
            path=str(REPO_ROOT),
            commit=_git(REPO_ROOT, "rev-parse", "HEAD"),
            dirty=bool(_git(REPO_ROOT, "status", "--porcelain")),
        ),
        vllm_fork=RepoEvidence(
            path=str(vllm_dir),
            commit=_git(vllm_dir, "rev-parse", "HEAD"),
            branch=_git(vllm_dir, "rev-parse", "--abbrev-ref", "HEAD"),
            origin_apxm=_git(vllm_dir, "rev-parse", "origin/apxm"),
            dirty=bool(_git(vllm_dir, "status", "--porcelain")),
        ),
        model=ModelEvidence(
            model_ref=_env_value(EnvVar.MODEL_REF),
            served_model_id=_env_value(EnvVar.SERVED_MODEL_ID),
            backend_name=_env_value(EnvVar.BACKEND_NAME),
            hf_home_host=_env_value(EnvVar.HF_HOME_HOST),
            port=_env_value(EnvVar.PORT),
        ),
        slurm=SlurmEvidence(
            job_id=_env_value(EnvVar.SLURM_JOB_ID),
            job_nodelist=_env_value(EnvVar.SLURM_JOB_NODELIST),
            job_partition=_env_value(EnvVar.SLURM_JOB_PARTITION),
        ),
        container=ContainerEvidence(
            image=image,
            inspect=_image_inspect(image),
        ),
        endpoint=EndpointEvidence(
            base_url=endpoint,
            models=_http_json(_endpoint_url(endpoint, ApiRoute.MODELS)),
            scheduler=_http_json(_endpoint_url(endpoint, ApiRoute.APXM_SCHEDULER)),
        ),
        host=HostEvidence(
            hostname=_command_json([ToolName.HOSTNAME.value]),
            gpu_smi=_command_json(
                enum_values(
                    [
                        ToolName.GPU_SMI,
                        RocmSmiFlag.SHOW_PRODUCT_NAME,
                        RocmSmiFlag.SHOW_MEMORY_INFO,
                        RocmSmiFlag.VRAM,
                        RocmSmiFlag.SHOW_USE,
                        RocmSmiFlag.SHOW_TEMP,
                        RocmSmiFlag.JSON,
                    ]
                ),
                timeout=60.0,
            ),
        ),
    )
    manifest_path.parent.mkdir(parents=True, exist_ok=True)
    manifest_path.write_text(
        json.dumps(asdict(manifest), indent=2, sort_keys=True) + "\n"
    )
    return manifest_path


def _summarize(records: list[RunRecord]) -> None:
    by_opt: dict[int, list[RunRecord]] = {}
    for record in records:
        by_opt.setdefault(record.opt_level, []).append(record)

    variants = sorted({record.variant for record in records})
    print("# benchmark_e2e summary")
    if variants != ["default"]:
        print(f"- variants: {', '.join(variants)}")
    # If any opt level produced critical-chain milestones, the workflow under
    # test cares about critical-chain finish time, not whole-graph wall time.
    # Promote the claim metric to its own line per opt level, alongside wall.
    has_critical_milestones = any(
        row.critical_milestone_last_ms is not None or row.observed_critical_path_finish_ms is not None
        for row in records
        if row.success
    )
    for opt_level in sorted(by_opt):
        rows = by_opt[opt_level]
        successes = [row for row in rows if row.success]
        wall_values = [row.wall_ms for row in successes]
        if wall_values:
            mean_wall = statistics.fmean(wall_values)
            median_wall = statistics.median(wall_values)
            graph_values = [
                row.graph_duration_ms for row in successes if row.graph_duration_ms is not None
            ]
            token_values = [
                row.total_tokens for row in successes if row.total_tokens is not None
            ]
            call_values = [
                row.llm_call_count for row in successes if row.llm_call_count is not None
            ]
            observed_cp_values = [
                row.observed_critical_path_ms
                for row in successes
                if row.observed_critical_path_ms is not None
            ]
            queue_wait_values = [
                row.queue_wait_total_ms
                for row in successes
                if row.queue_wait_total_ms is not None
            ]
            metric_parts = [
                f"mean wall {mean_wall:.1f} ms",
                f"median wall {median_wall:.1f} ms",
            ]
            if graph_values:
                metric_parts.append(f"mean graph {statistics.fmean(graph_values):.1f} ms")
            if token_values:
                metric_parts.append(f"mean tokens {statistics.fmean(token_values):.1f}")
            if call_values:
                metric_parts.append(f"mean LLM calls {statistics.fmean(call_values):.1f}")
            if observed_cp_values:
                metric_parts.append(
                    f"observed critical path {statistics.fmean(observed_cp_values):.1f} ms"
                )
            if queue_wait_values:
                metric_parts.append(
                    f"queue wait {statistics.fmean(queue_wait_values):.1f} ms"
                )
            print(
                f"- O{opt_level}: {len(successes)}/{len(rows)} succeeded, "
                + ", ".join(metric_parts)
            )
            if has_critical_milestones:
                milestone_values = [
                    row.critical_milestone_last_ms
                    for row in successes
                    if row.critical_milestone_last_ms is not None
                ]
                finish_values = [
                    row.observed_critical_path_finish_ms
                    for row in successes
                    if row.observed_critical_path_finish_ms is not None
                ]
                claim_parts: list[str] = []
                if milestone_values:
                    claim_parts.append(
                        f"mean critical milestone last {statistics.fmean(milestone_values):.1f} ms"
                    )
                if finish_values:
                    claim_parts.append(
                        f"mean observed critical finish {statistics.fmean(finish_values):.1f} ms"
                    )
                if claim_parts:
                    print(
                        f"  claim metric (critical-chain finish, not whole-graph wall): "
                        + ", ".join(claim_parts)
                    )
        else:
            print(f"- O{opt_level}: 0/{len(rows)} succeeded")
    # Cross-opt-level delta on the claim metric makes the priority-hints win
    # legible without scraping the CSV. Skip silently if we do not have at
    # least two opt levels with milestone data.
    if has_critical_milestones:
        per_opt_milestone: dict[int, float] = {}
        for opt_level in sorted(by_opt):
            successes = [row for row in by_opt[opt_level] if row.success]
            milestone_values = [
                row.critical_milestone_last_ms
                for row in successes
                if row.critical_milestone_last_ms is not None
            ]
            if milestone_values:
                per_opt_milestone[opt_level] = statistics.fmean(milestone_values)
        if len(per_opt_milestone) >= 2:
            baseline_opt = min(per_opt_milestone)
            baseline = per_opt_milestone[baseline_opt]
            deltas: list[str] = []
            for opt_level, value in per_opt_milestone.items():
                if opt_level == baseline_opt:
                    continue
                delta_ms = value - baseline
                pct = (delta_ms / baseline * 100.0) if baseline else 0.0
                deltas.append(
                    f"O{opt_level} vs O{baseline_opt}: {delta_ms:+.1f} ms ({pct:+.1f}%)"
                )
            if deltas:
                print("- claim-metric delta: " + "; ".join(deltas))


def _display_number(value: int | float | None, suffix: str = "") -> str:
    if value is None:
        return PENDING_VALUE
    if isinstance(value, int):
        return f"{value}{suffix}"
    return f"{value:.1f}{suffix}"


def _print_run_summary(record: RunRecord) -> None:
    status = STATUS_SUCCEEDED if record.success else STATUS_FAILED
    diagnostic_part = ""
    if record.compiler_diagnostics.diagnostics_path:
        diagnostic_part = (
            f"; passes={_display_number(record.compiler_diagnostics.pass_count)}"
            f"; dspy_fired={_display_number(record.compiler_diagnostics.dspy_fired_count)}"
        )
    compile_part = ""
    if record.compile_wall_ms is not None:
        compile_part = f"; compile={record.compile_wall_ms:.1f} ms"
    print(
        f"- {RUN_SUMMARY_PREFIX}: {record.mode} {record.variant} "
        f"O{record.opt_level}/{record.optimization_target} trial {record.trial_id} "
        f"run {record.run_index} order {record.run_order} {status}; "
        f"wall={record.wall_ms:.1f} ms; "
        f"graph={_display_number(record.graph_duration_ms, ' ms')}; "
        f"llm_calls={_display_number(record.llm_call_count)}; "
        f"tokens={_display_number(record.total_tokens)}; "
        f"session={record.session_dir or PENDING_VALUE}"
        f"{compile_part}"
        f"{diagnostic_part}",
        flush=True,
    )


def _print_precompile_summary(build: ArtifactBuild, target: str) -> None:
    status = STATUS_SUCCEEDED if build.success else STATUS_FAILED
    diagnostic_part = ""
    if build.compiler_diagnostics.diagnostics_path:
        diagnostic_part = (
            f"; passes={_display_number(build.compiler_diagnostics.pass_count)}"
            f"; dspy_fired={_display_number(build.compiler_diagnostics.dspy_fired_count)}"
        )
    print(
        f"- {PRECOMPILE_SUMMARY_PREFIX}: "
        f"O{build.opt_level}/{target} {status}; "
        f"wall={build.wall_ms:.1f} ms; "
        f"artifact={build.artifact_path}"
        f"{diagnostic_part}",
        flush=True,
    )


def main() -> int:
    args = _parse_args()
    _require_dekk()

    graph = args.graph.resolve()
    if not graph.exists():
        raise SystemExit(f"error: graph not found: {graph}")
    if args.iterations < 1:
        raise SystemExit("error: --iterations must be >= 1")
    if args.compile_only and args.precompile_artifacts:
        raise SystemExit("error: --compile-only and --precompile-artifacts are mutually exclusive")

    opt_levels = args.opt_levels or [0, 2]
    diagnostics_parent = (
        args.diagnostics_dir.resolve()
        if args.diagnostics_dir is not None
        else args.output.resolve().parent / DIR_COMPILER_DIAGNOSTICS
    )
    artifact_builds: dict[int, ArtifactBuild] = {}
    if args.precompile_artifacts:
        artifact_dir = (
            args.artifact_dir.resolve()
            if args.artifact_dir is not None
            else args.output.resolve().parent / DIR_ARTIFACTS / args.output.resolve().stem
        )
        artifact_builds = _precompile_artifacts(
            graph=graph,
            opt_levels=opt_levels,
            target=args.target,
            artifact_dir=artifact_dir,
            diagnostics_parent=diagnostics_parent,
            apxm_config=args.apxm_config.resolve() if args.apxm_config else None,
            emit_compiler_diagnostics=args.emit_compiler_diagnostics,
        )
    records: list[RunRecord] = []
    run_order = 0

    if args.interleave_opt_levels:
        run_plan = [
            (opt_level, trial_id, trial_id)
            for trial_id in range(1, args.iterations + 1)
            for opt_level in opt_levels
        ]
    else:
        run_plan = [
            (opt_level, run_index, run_index)
            for opt_level in opt_levels
            for run_index in range(1, args.iterations + 1)
        ]

    for opt_level, run_index, trial_id in run_plan:
        run_order += 1
        record = _run_once(
            graph=graph,
            opt_level=opt_level,
            target=args.target,
            run_index=run_index,
            trial_id=trial_id,
            run_order=run_order,
            compile_only=args.compile_only,
            variant=args.variant,
            backend_label=args.backend_label,
            trace=args.trace,
            session_parent=args.session_base.resolve(),
            apxm_config=args.apxm_config.resolve() if args.apxm_config else None,
            emit_compiler_diagnostics=args.emit_compiler_diagnostics,
            diagnostics_parent=diagnostics_parent,
            artifact_build=artifact_builds.get(opt_level),
            runtime_args=args.runtime_args,
        )
        records.append(record)
        _print_run_summary(record)

    _write_csv(records, args.output.resolve(), args.append)
    evidence_path = _write_evidence_manifest(
        args=args,
        records=records,
        output=args.output.resolve(),
    )
    _summarize(records)
    print(f"CSV written to {args.output.resolve()}")
    print(f"Evidence manifest written to {evidence_path}")

    return 0 if all(record.success for record in records) else 1


if __name__ == "__main__":
    sys.exit(main())
