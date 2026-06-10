#!/usr/bin/env python3
"""Shared APXM/vLLM controller contract.

This module owns the operational names used by the Dekk vLLM controller and
the checked-in vLLM examples. Keep machine-specific policy out of callers.
"""

from __future__ import annotations

import os
from dataclasses import dataclass
from enum import Enum
from pathlib import Path


class EnvVar(str, Enum):
    """Environment variables consumed by the APXM/vLLM path."""

    APXM_CONFIG = "APXM_CONFIG"
    APXM_DISABLE_HINTS = "APXM_DISABLE_HINTS"
    APXM_ENDPOINT = "APXM_ENDPOINT"
    APXM_HOME = "APXM_HOME"
    APXM_MATRIX_VARIANT = "APXM_MATRIX_VARIANT"
    APXM_VLLM_CACHE_SALT = "APXM_VLLM_CACHE_SALT"
    APXM_VLLM_HF_CACHE_ROOTS = "APXM_VLLM_HF_CACHE_ROOTS"
    APXM_VLLM_HF_HOME = "APXM_VLLM_HF_HOME"
    APXM_VLLM_IMAGE = "APXM_VLLM_IMAGE"
    APXM_VLLM_IMAGE_STORE = "APXM_VLLM_IMAGE_STORE"
    APXM_VLLM_MODEL_ROOTS = "APXM_VLLM_MODEL_ROOTS"
    APXM_VLLM_SERVICE_NAME = "APXM_VLLM_SERVICE_NAME"
    BACKEND_NAME = "BACKEND_NAME"
    CUDA_VISIBLE_DEVICES = "CUDA_VISIBLE_DEVICES"
    ENABLE_PREFIX_CACHING = "ENABLE_PREFIX_CACHING"
    GPUS = "GPUS"
    HF_HOME_HOST = "HF_HOME_HOST"
    HF_HOME = "HF_HOME"
    HUGGINGFACE_HUB_CACHE = "HUGGINGFACE_HUB_CACHE"
    HF_TOKEN = "HF_TOKEN"
    HIP_VISIBLE_DEVICES = "HIP_VISIBLE_DEVICES"
    MAX_MODEL_LEN = "MAX_MODEL_LEN"
    MAX_NUM_SEQS = "MAX_NUM_SEQS"
    MODEL_REF = "MODEL_REF"
    PORT = "PORT"
    REASONING_PARSER = "REASONING_PARSER"
    SCHEDULING_POLICY = "SCHEDULING_POLICY"
    TOOL_CALL_PARSER = "TOOL_CALL_PARSER"
    ENABLE_AUTO_TOOL_CHOICE = "ENABLE_AUTO_TOOL_CHOICE"
    SERVED_MODEL_ID = "SERVED_MODEL_ID"
    SLURM_JOB_ID = "SLURM_JOB_ID"
    SLURM_JOB_NODELIST = "SLURM_JOB_NODELIST"
    SLURM_JOB_PARTITION = "SLURM_JOB_PARTITION"
    STARTUP_TIMEOUT_SECONDS = "STARTUP_TIMEOUT_SECONDS"
    TENSOR_PARALLEL_SIZE = "TENSOR_PARALLEL_SIZE"
    VLLM_API_KEY = "VLLM_API_KEY"
    XDG_CACHE_HOME = "XDG_CACHE_HOME"


class BackendType(str, Enum):
    """APXM backend type values used by `dekk apxm backend add`."""

    LOCAL = "local"
    ON_PREM = "onprem"


class BackendProtocol(str, Enum):
    """APXM backend protocol values used by `dekk apxm backend add`."""

    VLLM = "vllm"


class DekkToken(str, Enum):
    """Command tokens for invoking APXM through Dekk."""

    DEKK = "dekk"
    APXM = "apxm"
    BACKEND = "backend"
    ADD = "add"
    ADD_MODEL = "add-model"
    TEST = "test"


class ApiRoute(str, Enum):
    """OpenAI-compatible and APXM vLLM route fragments."""

    OPENAI_PREFIX = "v1"
    CHAT_COMPLETIONS = "chat/completions"
    MODELS = "models"
    APXM_GRAPHS = "apxm/graphs"
    APXM_REGISTER = "register"
    APXM_SCHEDULER = "apxm/scheduler"


class ArmName(str, Enum):
    """Per-arm sentinels used by A/B benchmark drivers to label runs."""

    APXM_ON = "apxm-on"
    FLAT_HTTP = "flat-http"


class WireKey(str, Enum):
    """Keys carried inside `extra_body` payloads on the OpenAI-compatible wire."""

    REUSE_GROUP = "reuse_group"
    PIN_POLICY = "pin_policy"
    PIN_MODE = "mode"
    PIN_PREFIX = "prefix"
    PIN_TTL_MS = "ttl_ms"
    CACHE_SALT = "cache_salt"


class HttpHeader(str, Enum):
    """HTTP header names used by the APXM/vLLM controller."""

    AUTHORIZATION = "authorization"
    CONTENT_TYPE = "content-type"


class HttpMethod(str, Enum):
    """HTTP methods used by the APXM/vLLM controller."""

    DELETE = "DELETE"
    GET = "GET"
    POST = "POST"


class MediaType(str, Enum):
    """HTTP media types used by APXM/vLLM JSON endpoints."""

    JSON = "application/json"


class AuthScheme(str, Enum):
    """HTTP auth schemes emitted by the controller."""

    BEARER = "Bearer"


class HostAddress(str, Enum):
    """Host names used for local APXM-vLLM endpoint registration."""

    LOOPBACK = "127.0.0.1"
    LOCALHOST = "localhost"
    ANY = "0.0.0.0"
    ANY_V6 = "::"


class ForkModule(str, Enum):
    """Python module names that prove the repo-local fork is in use."""

    VLLM = "vllm"
    APXM_ROUTER = "vllm.entrypoints.openai.apxm.api_router"
    OPENAI_API_SERVER = "vllm.entrypoints.openai.api_server"
    VLLM_CLI = "vllm.entrypoints.cli.main"


class ToolName(str, Enum):
    """External command names used by the controller."""

    GIT = "git"
    DOCKER = "docker"
    HOSTNAME = "hostname"
    LSOF = "lsof"
    SBATCH = "sbatch"
    SCANCEL = "scancel"
    SINFO = "sinfo"
    SQUEUE = "squeue"
    SRUN = "srun"


class VllmCommand(str, Enum):
    """Subcommands exposed under `dekk apxm vllm`."""

    HELP = "help"
    DOCTOR = "doctor"
    PROBE = "probe"
    ENABLE = "enable"
    CACHE_WARM = "cache-warm"
    DOCKER_BUILD = "docker-build"
    DOCKER_LOAD = "docker-load"
    DOCKER_START = "docker-start"
    DOCKER_SAVE = "docker-save"
    DOCKER_STOP = "docker-stop"
    DOCKER_STATUS = "docker-status"
    DOCKER_LOGS = "docker-logs"
    SERVICE_STATUS = "service-status"
    SERVICE_LIST = "service-list"
    SERVICE_EXEC = "service-exec"
    SERVICE_STOP = "service-stop"
    ZOO_APPLY = "zoo-apply"
    ZOO_STATUS = "zoo-status"
    ZOO_SCALE = "zoo-scale"
    ZOO_CACHE_WARM = "zoo-cache-warm"
    ZOO_LOGS = "zoo-logs"


class DockerCommand(str, Enum):
    """Docker subcommands used by the APXM/vLLM controller."""

    BUILD = "build"
    BUILDX = "buildx"
    IMAGE = "image"
    INFO = "info"
    INSPECT = "inspect"
    LOAD = "load"
    LOGS = "logs"
    RM = "rm"
    RUN = "run"
    SAVE = "save"


class DockerBuildxCommand(str, Enum):
    """Docker buildx subcommands used by the APXM/vLLM controller."""

    BUILD = "build"
    VERSION = "version"


class DockerFlag(str, Enum):
    """Docker flags used by the APXM/vLLM controller."""

    BUILD_ARG = "--build-arg"
    DETACH = "-d"
    DEVICE = "--device"
    ENV = "-e"
    FILE = "-f"
    FORCE = "-f"
    FORMAT = "--format"
    GROUP_ADD = "--group-add"
    INPUT = "-i"
    IPC_HOST = "--ipc=host"
    LABEL = "--label"
    LOAD = "--load"
    NAME = "--name"
    NETWORK = "--network"
    REMOVE = "--rm"
    SECURITY_OPT = "--security-opt"
    TAG = "-t"
    TAIL = "--tail"
    VOLUME = "-v"
    OUTPUT = "-o"
    FOLLOW = "-f"


class DockerValue(str, Enum):
    """Structured Docker argument values that are part of the APXM runtime contract."""

    HOST_NETWORK = "host"
    IMAGE_ID_FORMAT = "{{json .Id}}"
    IMAGE_METADATA_FORMAT = "{{json .}}"
    JSON_OBJECT_FORMAT = "{{json .}}"
    RUNNING_FORMAT = "{{.State.Running}}"
    STATUS_FORMAT = "name={{.Name}} id={{.Id}} running={{.State.Running}} status={{.State.Status}} image={{.Config.Image}}"
    SECCOMP_UNCONFINED = "seccomp=unconfined"


class DockerLabel(str, Enum):
    """Docker label names applied to Dekk-managed APXM-vLLM containers."""

    MANAGED_BY = "apxm.managed_by"
    SLURM_JOB_ID = "apxm.slurm_job_id"
    BACKEND_NAME = "apxm.backend_name"
    SERVED_MODEL_ID = "apxm.served_model_id"


class ContainerPath(str, Enum):
    """Container paths used by the APXM/vLLM Docker runtime."""

    HF_HOME = "/models/hf"
    MODEL_ROOTS = "/models/roots"


class ApxmWorkspacePath(str, Enum):
    """Repo-local APXM workspace path segments."""

    ROOT = ".apxm"
    CONFIG = "config.toml"
    VLLM_LOGS = "vllm-logs"
    VLLM_IMAGES = "vllm-images"
    VLLM_SERVICES = "vllm-services"
    DEPLOY = "deploy"
    BENCHMARKS = "benchmarks"
    EVALUATION = "evaluation"
    GEMMA4 = "gemma4"
    RESULTS = "results"
    RUNS = "runs"


class RepoMarker(str, Enum):
    """Files and directories that identify an APXM repository root."""

    CARGO_TOML = "Cargo.toml"
    CRATES = "crates"
    GIT = ".git"


class RepoPath(str, Enum):
    """Repo-relative source path segments used by APXM controller scripts."""

    EXTERNAL = "external"
    VLLM = "vllm"


class ArgName(str, Enum):
    """argparse destination names used by the controller."""

    ALIAS = "alias"
    API_KEY = "api_key"
    API_KEY_ENV = "api_key_env"
    ARCHIVE = "archive"
    BASE_IMAGE = "base_image"
    COMMAND = "command"
    CONTAINER_ENV = "container_env"
    CONTAINER_NAME = "container_name"
    DEFAULT_CHAT_TEMPLATE_KWARGS = "default_chat_template_kwargs"
    DOCKERFILE = "dockerfile"
    ENDPOINT = "endpoint"
    ENABLE_FORCE_INCLUDE_USAGE = "enable_force_include_usage"
    ENABLE_PREFIX_CACHING = "enable_prefix_caching"
    ENABLE_PROMPT_TOKENS_DETAILS = "enable_prompt_tokens_details"
    GPUS = "gpus"
    HANDLER = "handler"
    HF_HOME = "hf_home"
    IMAGE = "image"
    MANIFEST = "manifest"
    MAX_NUM_SEQS = "max_num_seqs"
    MODEL = "model"
    MODEL_ROOT = "model_root"
    NAME = "name"
    PORT = "port"
    PRUNE = "prune"
    REASONING_PARSER = "reasoning_parser"
    REPLICAS = "replicas"
    TOOL_CALL_PARSER = "tool_call_parser"
    ENABLE_AUTO_TOOL_CHOICE = "enable_auto_tool_choice"
    SCHEDULING_POLICY = "scheduling_policy"
    SERVED_MODEL_NAME = "served_model_name"
    TENSOR_PARALLEL_SIZE = "tensor_parallel_size"
    WAIT = "wait"


class VllmServeFlag(str, Enum):
    """vLLM server CLI flags forwarded by the controller."""

    DEFAULT_CHAT_TEMPLATE_KWARGS = "--default-chat-template-kwargs"
    ENABLE_AUTO_TOOL_CHOICE = "--enable-auto-tool-choice"
    ENABLE_FORCE_INCLUDE_USAGE = "--enable-force-include-usage"
    ENABLE_PREFIX_CACHING = "--enable-prefix-caching"
    ENABLE_PROMPT_TOKENS_DETAILS = "--enable-prompt-tokens-details"
    REASONING_PARSER = "--reasoning-parser"
    SCHEDULING_POLICY = "--scheduling-policy"
    TOOL_CALL_PARSER = "--tool-call-parser"


class SchedulingPolicy(str, Enum):
    """vLLM scheduler policy values the APXM controller will emit.

    Single-variant enum: the upstream fork still implements the FCFS branch,
    but APXM never selects it — per-request critical-path boosts only take
    effect under PRIORITY mode, and the zoo manifest disallows anything
    else. The upstream FCFS code path is documented as not exercised by
    APXM.
    """

    PRIORITY = "priority"


@dataclass(frozen=True)
class VllmDefaults:
    """Controller defaults that are model-neutral and safe for local hosts."""

    backend_name: str = "vllm-fork"
    host: str = HostAddress.LOOPBACK.value
    # Default local service port for single-service benchmark helpers. The
    # model-zoo allocator still owns real service port selection.
    port: int = 8916
    request_timeout_seconds: int = 15
    startup_timeout_seconds: int = 900
    stop_timeout_seconds: float = 20.0
    download_workers: int = 8
    log_lines: int = 80
    # APXM ships with priority on by default so compiler-stamped critical-path
    # hints actually reorder the waiting queue.
    scheduling_policy: str = SchedulingPolicy.PRIORITY.value
    # APXM ships with prompt_tokens_details on by default so the OpenAI parser
    # at apxm-backends/src/llm/backends/openai/backend.rs:779-789 can populate
    # cached_input_tokens per request from usage.prompt_tokens_details.cached_tokens.
    # The fork's emission is gated at vllm/entrypoints/openai/chat_completion/serving.py
    # by --enable-prompt-tokens-details (default False upstream). Operators can
    # opt out with --no-enable-prompt-tokens-details.
    enable_prompt_tokens_details: bool = True


@dataclass(frozen=True)
class SlurmServiceDefaults:
    """Defaults exported by the zoo apply path to the unified Slurm wrapper."""

    max_model_len: int = 32768
    max_num_seqs: int = 64
    scheduling_policy: str = SchedulingPolicy.PRIORITY.value
    enable_prefix_caching: bool = True
    startup_timeout_seconds: float = 7200.0


@dataclass(frozen=True)
class ProbeContract:
    """Temporary graph ids and node names used by `dekk apxm vllm probe`."""

    graph_id: str = "__apxm_probe__"
    temp_graph_id_prefix: str = "dekk-probe"
    temp_execution_id_prefix: str = "dekk-probe-exec"
    temp_node_name: str = "dekk-probe"


@dataclass(frozen=True)
class RepoLayout:
    """Resolved repo paths for APXM-vLLM operations."""

    repo_root: Path
    workspace_dir: Path
    vllm_dir: Path
    log_dir: Path
    image_store_dir: Path
    service_dir: Path
    benchmark_results_dir: Path
    evaluation_dir: Path
    gemma4_runs_dir: Path


def enum_value(value: str | Enum) -> str:
    return value.value if isinstance(value, Enum) else value


def enum_values(values: list[str | Enum]) -> list[str]:
    return [enum_value(value) for value in values]


def arg_value(namespace: object, arg_name: ArgName, default: object = None) -> object:
    return getattr(namespace, arg_name.value, default)


def env_name(env_var: EnvVar) -> str:
    return env_var.value


def env_reference(env_var: EnvVar | str) -> str:
    return f"env:{enum_value(env_var)}"


def find_repo_root(start: str | Path) -> Path:
    start_path = Path(start).resolve()
    if start_path.is_file():
        start_path = start_path.parent
    for candidate in (start_path, *start_path.parents):
        if (
            (candidate / ApxmWorkspacePath.ROOT.value).is_dir()
            or (
                (candidate / RepoMarker.CARGO_TOML.value).is_file()
                and (candidate / RepoMarker.CRATES.value).is_dir()
            )
            or (candidate / RepoMarker.GIT.value).exists()
        ):
            return candidate
    raise RuntimeError(f"could not locate APXM repo root from {start_path}")


def workspace_path(repo_root: Path, *segments: ApxmWorkspacePath | str) -> Path:
    path = repo_root / ApxmWorkspacePath.ROOT.value
    for segment in segments:
        path = path / enum_value(segment)
    return path


def build_layout(script_file: str | Path) -> RepoLayout:
    repo_root = find_repo_root(script_file)
    workspace_dir = workspace_path(repo_root)
    evaluation_dir = workspace_path(repo_root, ApxmWorkspacePath.EVALUATION)
    vllm_dir = repo_root / RepoPath.EXTERNAL.value / RepoPath.VLLM.value
    # Roaming buckets are relocatable via .apxm/config.toml; pinned
    # buckets always live at <repo>/.apxm/.
    from apxm.data_config import resolve_data_layout

    data_layout = resolve_data_layout(repo_root)
    return RepoLayout(
        repo_root=repo_root,
        workspace_dir=workspace_dir,
        vllm_dir=vllm_dir,
        log_dir=workspace_path(repo_root, ApxmWorkspacePath.VLLM_LOGS),
        image_store_dir=data_layout.image_store,
        service_dir=workspace_path(repo_root, ApxmWorkspacePath.VLLM_SERVICES),
        benchmark_results_dir=workspace_path(
            repo_root,
            ApxmWorkspacePath.BENCHMARKS,
            ApxmWorkspacePath.RESULTS,
        ),
        evaluation_dir=evaluation_dir,
        gemma4_runs_dir=workspace_path(
            repo_root,
            ApxmWorkspacePath.EVALUATION,
            ApxmWorkspacePath.GEMMA4,
            ApxmWorkspacePath.RUNS,
        ),
    )


def apxm_config_path(start: Path | None = None) -> Path:
    explicit = os.environ.get(env_name(EnvVar.APXM_CONFIG), "").strip()
    if explicit:
        return Path(explicit)

    cwd = (start or Path.cwd()).resolve()
    for candidate_root in (cwd, *cwd.parents):
        candidate = workspace_path(candidate_root, ApxmWorkspacePath.CONFIG)
        if candidate.is_file():
            return candidate

    return Path.home() / ".apxm" / "config.toml"


def effective_hf_home(
    *,
    environ: dict[str, str] | os._Environ[str] = os.environ,
) -> str:
    """Resolve the HF cache root via the shared data-layout chain;
    ``dekk apxm vllm doctor`` prints the resolved value and its source."""
    from apxm.data_config import resolve_data_layout

    layout = resolve_data_layout(find_repo_root(Path(__file__)), environ=environ)
    return str(layout.hf_cache)


def effective_hf_cache_roots(
    *,
    environ: dict[str, str] | os._Environ[str] = os.environ,
) -> tuple[str, ...]:
    """Return the ordered HF cache roots APXM may search for cached models.

    The first element is always the writable primary ``hf_cache``; remaining
    roots are read-only search locations configured via
    ``data.vllm.hf_cache_roots`` or ``APXM_VLLM_HF_CACHE_ROOTS``.
    """
    from apxm.data_config import resolve_data_layout

    layout = resolve_data_layout(find_repo_root(Path(__file__)), environ=environ)
    return tuple(str(path) for path in layout.hf_cache_roots)


def effective_model_roots(
    *,
    environ: dict[str, str] | os._Environ[str] = os.environ,
) -> tuple[str, ...]:
    """Return host directories that APXM bind-mounts for local model paths."""
    from apxm.data_config import resolve_data_layout

    layout = resolve_data_layout(find_repo_root(Path(__file__)), environ=environ)
    return tuple(str(path) for path in layout.model_roots)


def local_endpoint(*, host: str, port: int) -> str:
    return f"http://{host}:{port}/{ApiRoute.OPENAI_PREFIX.value}"


def openai_route_path(route: ApiRoute | str) -> str:
    return f"/{ApiRoute.OPENAI_PREFIX.value}/{enum_value(route)}"


def graph_register_path() -> str:
    return f"{ApiRoute.APXM_GRAPHS.value}/{ApiRoute.APXM_REGISTER.value}"
