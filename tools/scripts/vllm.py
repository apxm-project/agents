#!/usr/bin/env python3
"""Operate APXM-vLLM through Dekk.

The canonical path is a persistent Slurm-owned service launched by Dekk. Docker
isolates the vLLM server process inside that allocation; APXM commands should
reach it through `service-exec` unless the operator already owns the allocation.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import socket
import subprocess
import sys
import time
import tomllib
import urllib.error
import urllib.request
import uuid
from pathlib import Path
from typing import Any

from apxm_data_config import (
    format_layout as format_data_layout,
    materialize_config as materialize_data_config,
    resolve_data_layout,
)
from apxm_vllm_contract import (
    ApiRoute,
    ArgName,
    AuthScheme,
    BackendProtocol,
    BackendType,
    ContainerPath,
    DekkToken,
    DockerBuildxCommand,
    DockerCommand,
    DockerFlag,
    DockerLabel,
    DockerValue,
    EnvVar,
    ForkModule,
    HostAddress,
    HttpHeader,
    HttpMethod,
    MediaType,
    ProbeContract,
    SchedulingPolicy,
    SlurmServiceDefaults,
    ToolName,
    VllmCommand,
    VllmDefaults,
    VllmServeFlag,
    apxm_config_path,
    arg_value,
    build_layout,
    effective_hf_home,
    enum_values,
    env_name,
    env_reference,
    graph_register_path,
    local_endpoint,
    openai_route_path,
)

LAYOUT = build_layout(__file__)
DEFAULTS = VllmDefaults()
SERVICE_DEFAULTS = SlurmServiceDefaults()
PROBE = ProbeContract()
REPO_ROOT = LAYOUT.repo_root
VLLM_DIR = LAYOUT.vllm_dir
LOG_DIR = LAYOUT.log_dir
IMAGE_STORE_DIR = LAYOUT.image_store_dir
SERVICE_DIR = LAYOUT.service_dir
APXM_CONFIG = apxm_config_path(REPO_ROOT)

DEFAULT_BACKEND_NAME = DEFAULTS.backend_name
DEFAULT_HOST = DEFAULTS.host
DEFAULT_REQUEST_TIMEOUT_SECONDS = DEFAULTS.request_timeout_seconds
DEFAULT_STARTUP_TIMEOUT_SECONDS = DEFAULTS.startup_timeout_seconds
DEFAULT_STOP_TIMEOUT_SECONDS = DEFAULTS.stop_timeout_seconds
LOCALHOST = HostAddress.LOOPBACK.value
LOCALHOST_NAME = HostAddress.LOCALHOST.value
ANY_HOST = HostAddress.ANY.value
ANY_HOST_V6 = HostAddress.ANY_V6.value
MODELS_PATH = ApiRoute.MODELS.value
APXM_GRAPHS_PATH = ApiRoute.APXM_GRAPHS.value
APXM_SCHEDULER_PATH = ApiRoute.APXM_SCHEDULER.value
APXM_GRAPH_REGISTER_PATH = graph_register_path()
PROBE_GRAPH_ID = PROBE.graph_id
TEMP_GRAPH_ID_PREFIX = PROBE.temp_graph_id_prefix
TEMP_EXECUTION_ID_PREFIX = PROBE.temp_execution_id_prefix
TEMP_NODE_NAME = PROBE.temp_node_name
ENV_HF_HOME = env_name(EnvVar.HF_HOME)
ENV_APXM_VLLM_HF_HOME = env_name(EnvVar.APXM_VLLM_HF_HOME)
ENV_APXM_VLLM_IMAGE = env_name(EnvVar.APXM_VLLM_IMAGE)
ENV_APXM_VLLM_SERVICE_NAME = env_name(EnvVar.APXM_VLLM_SERVICE_NAME)
ENV_HF_TOKEN = env_name(EnvVar.HF_TOKEN)
ENV_VLLM_API_KEY = env_name(EnvVar.VLLM_API_KEY)
ENV_HIP_VISIBLE_DEVICES = env_name(EnvVar.HIP_VISIBLE_DEVICES)
ENV_CUDA_VISIBLE_DEVICES = env_name(EnvVar.CUDA_VISIBLE_DEVICES)
ENV_MODEL_REF = env_name(EnvVar.MODEL_REF)
ENV_SERVED_MODEL_ID = env_name(EnvVar.SERVED_MODEL_ID)
ENV_BACKEND_NAME = env_name(EnvVar.BACKEND_NAME)
ENV_PORT = env_name(EnvVar.PORT)
ENV_HF_HOME_HOST = env_name(EnvVar.HF_HOME_HOST)
ENV_MAX_MODEL_LEN = env_name(EnvVar.MAX_MODEL_LEN)
ENV_MAX_NUM_SEQS = env_name(EnvVar.MAX_NUM_SEQS)
ENV_SCHEDULING_POLICY = env_name(EnvVar.SCHEDULING_POLICY)
ENV_ENABLE_PREFIX_CACHING = env_name(EnvVar.ENABLE_PREFIX_CACHING)
ENV_STARTUP_TIMEOUT_SECONDS = env_name(EnvVar.STARTUP_TIMEOUT_SECONDS)
ENV_TENSOR_PARALLEL_SIZE = env_name(EnvVar.TENSOR_PARALLEL_SIZE)
ENV_GPUS = env_name(EnvVar.GPUS)
ENV_SLURM_JOB_ID = env_name(EnvVar.SLURM_JOB_ID)
ENV_SLURM_JOB_NODELIST = env_name(EnvVar.SLURM_JOB_NODELIST)
PORT_ALLOCATOR_MIN = 8916
PORT_ALLOCATOR_MAX = 8999
STATE_VERSION = 1
GIT = ToolName.GIT.value
DOCKER = ToolName.DOCKER.value
LSOF = ToolName.LSOF.value
SBATCH = ToolName.SBATCH.value
SCANCEL = ToolName.SCANCEL.value
SINFO = ToolName.SINFO.value
SQUEUE = ToolName.SQUEUE.value
SRUN = ToolName.SRUN.value
MANAGED_BY = "dekk apxm vllm"
APXM_ROUTER_FILE = VLLM_DIR / "vllm" / "entrypoints" / "openai" / "apxm" / "api_router.py"
OPENAI_API_SERVER_FILE = VLLM_DIR / "vllm" / "entrypoints" / "openai" / "api_server.py"
COMMANDS_WITHOUT_EXTRA_ARGS = {
    VllmCommand.HELP.value,
    VllmCommand.DOCTOR.value,
    VllmCommand.PROBE.value,
    VllmCommand.ENABLE.value,
    VllmCommand.CACHE_WARM.value,
    VllmCommand.DOCKER_BUILD.value,
    VllmCommand.DOCKER_LOAD.value,
    VllmCommand.DOCKER_STOP.value,
    VllmCommand.DOCKER_STATUS.value,
    VllmCommand.DOCKER_LOGS.value,
    VllmCommand.DOCKER_SAVE.value,
    VllmCommand.SERVICE_START.value,
    VllmCommand.SERVICE_STATUS.value,
    VllmCommand.SERVICE_LIST.value,
    VllmCommand.SERVICE_STOP.value,
    VllmCommand.ZOO_APPLY.value,
    VllmCommand.ZOO_STATUS.value,
    VllmCommand.ZOO_SCALE.value,
    VllmCommand.ZOO_CACHE_WARM.value,
    VllmCommand.ZOO_LOGS.value,
}


def _print(msg: str) -> None:
    print(msg, file=sys.stderr)


def _run(
    cmd: list[str],
    *,
    cwd: Path | None = None,
    env: dict[str, str] | None = None,
) -> int:
    result = subprocess.run(cmd, cwd=cwd, env=env)
    return result.returncode


def _capture(
    cmd: list[str],
    *,
    cwd: Path | None = None,
    env: dict[str, str] | None = None,
) -> subprocess.CompletedProcess[str]:
    return subprocess.run(cmd, cwd=cwd, env=env, capture_output=True, text=True)


def _capture_stdout(cmd: list[str], *, cwd: Path | None = None) -> str:
    result = _capture(cmd, cwd=cwd)
    if result.returncode != 0:
        return ""
    return result.stdout.strip()


def _git_stdout(repo: Path, *args: str) -> str:
    return _capture_stdout([GIT, "-C", str(repo), *args])


def _git_dirty(repo: Path) -> bool:
    return bool(_git_stdout(repo, "status", "--porcelain"))


def _tool_path(tool: str) -> str:
    return shutil.which(tool) or ""


def _check_line(status: str, name: str, detail: str = "") -> None:
    suffix = f" {detail}" if detail else ""
    print(f"{status}: {name}{suffix}")


def _verify_fork_source(*, verbose: bool) -> bool:
    errors = 0
    if not (VLLM_DIR / "pyproject.toml").exists():
        _check_line("ERROR", "external/vllm source", f"missing pyproject.toml at {VLLM_DIR}")
        return False

    checks = (
        (
            "APXM router file",
            APXM_ROUTER_FILE,
            (
                openai_route_path(APXM_GRAPH_REGISTER_PATH),
                openai_route_path(ApiRoute.APXM_SCHEDULER),
            ),
        ),
        (
            "OpenAI API server mounts APXM router",
            OPENAI_API_SERVER_FILE,
            (ForkModule.APXM_ROUTER.value, "register_apxm_api_router"),
        ),
    )
    for label, path, needles in checks:
        try:
            text = path.read_text(encoding="utf-8")
        except OSError as exc:
            _check_line("ERROR", label, str(exc))
            errors += 1
            continue
        missing = [needle for needle in needles if needle not in text]
        if missing:
            _check_line("ERROR", label, f"missing {', '.join(missing)}")
            errors += 1
        elif verbose:
            _check_line("OK", label, str(path.relative_to(REPO_ROOT)))

    if verbose:
        commit = _git_stdout(VLLM_DIR, "rev-parse", "HEAD")
        branch = _git_stdout(VLLM_DIR, "rev-parse", "--abbrev-ref", "HEAD")
        origin_apxm = _git_stdout(VLLM_DIR, "rev-parse", "origin/apxm")
        _check_line("OK", "external/vllm commit", commit or "<unknown>")
        _check_line("OK", "external/vllm branch", branch or "<unknown>")
        if origin_apxm:
            status = "OK" if commit == origin_apxm else "WARN"
            _check_line(status, "external/vllm origin/apxm", origin_apxm)
        _check_line("WARN" if _git_dirty(VLLM_DIR) else "OK", "external/vllm dirty", str(_git_dirty(VLLM_DIR)).lower())

    return errors == 0


def _fork_contract_payload() -> str:
    return json.dumps(
        {
            "vllm_module": ForkModule.VLLM.value,
            "router_module": ForkModule.APXM_ROUTER.value,
            "api_server_module": ForkModule.OPENAI_API_SERVER.value,
        },
        sort_keys=True,
    )


def _doctor_payload() -> str:
    return json.dumps(
        {
            "modules": [
                ForkModule.VLLM.value,
                "torch",
                "transformers",
                "huggingface_hub",
            ],
            "router_module": ForkModule.APXM_ROUTER.value,
        },
        sort_keys=True,
    )


def _hf_home(args: argparse.Namespace | None = None) -> str:
    """Return the HF cache root from the single mandatory env var.

    The `args` parameter is kept for call-site compatibility but ignored;
    `APXM_VLLM_HF_HOME` is the only accepted source.
    """
    del args
    return effective_hf_home()


def _api_key(args: argparse.Namespace) -> str | None:
    """Resolve the vLLM API key.

    Three explicit, non-overlapping sources, in precedence order: the
    `--api-key` flag, the env var named by `--api-key-env`, and the
    well-known `VLLM_API_KEY`. Returns `None` if none are set — that is
    legitimate (a local-only vLLM instance often runs without auth) and
    is not the same as a missing required value.
    """
    explicit = arg_value(args, ArgName.API_KEY)
    if explicit:
        return explicit
    api_key_env = arg_value(args, ArgName.API_KEY_ENV)
    if api_key_env:
        named = os.environ.get(api_key_env)
        if named:
            return named
    fallback_env = os.environ.get(ENV_VLLM_API_KEY)
    if fallback_env:
        return fallback_env
    return None


def _api_key_config_reference(args: argparse.Namespace) -> str | None:
    api_key_env = arg_value(args, ArgName.API_KEY_ENV)
    if api_key_env:
        return env_reference(api_key_env)
    if os.environ.get(ENV_VLLM_API_KEY):
        return env_reference(EnvVar.VLLM_API_KEY)
    return None


def _endpoint(port: int) -> str:
    return local_endpoint(host=LOCALHOST, port=port)


def _endpoint_for_args(args: argparse.Namespace) -> str:
    explicit = arg_value(args, ArgName.ENDPOINT)
    if explicit:
        return _normalize_endpoint(explicit)
    port = arg_value(args, ArgName.PORT)
    if port is None:
        raise SystemExit(
            "endpoint resolution requires either --endpoint or --port."
        )
    return _normalize_endpoint(_endpoint(port))


def _normalize_endpoint(endpoint: str) -> str:
    normalized = endpoint.rstrip("/")
    normalized = normalized.replace(f"//{LOCALHOST_NAME}:", f"//{LOCALHOST}:")
    if not normalized.endswith(f"/{ApiRoute.OPENAI_PREFIX.value}"):
        normalized = f"{normalized}/{ApiRoute.OPENAI_PREFIX.value}"
    return normalized


def _temporary_id(prefix: str) -> str:
    return f"{prefix}-{uuid.uuid4()}"


def _container_state_file(port: int) -> Path:
    return LOG_DIR / f"container-{port}.json"


def _docker_available() -> bool:
    return (
        shutil.which(DOCKER) is not None
        and _capture([DOCKER, DockerCommand.INFO.value]).returncode == 0
    )


def _docker_buildx_available() -> bool:
    return (
        shutil.which(DOCKER) is not None
        and _capture(
            [DOCKER, DockerCommand.BUILDX.value, DockerBuildxCommand.VERSION.value]
        ).returncode
        == 0
    )


def _docker_capture(args: list[str]) -> subprocess.CompletedProcess[str]:
    return _capture([DOCKER, *args], cwd=REPO_ROOT)


def _docker_container_running(name: str) -> bool:
    result = _docker_capture(
        enum_values(
            [
                DockerCommand.INSPECT,
                DockerFlag.FORMAT,
                DockerValue.RUNNING_FORMAT,
                name,
            ]
        )
    )
    return result.returncode == 0 and result.stdout.strip().lower() == "true"


def _write_container_state(
    *,
    port: int,
    container_name: str,
    container_id: str,
    image: str,
    endpoint: str,
    model: str,
    served_model_name: str,
    backend_name: str,
    command: list[str],
) -> None:
    LOG_DIR.mkdir(parents=True, exist_ok=True)
    state = {
        "version": STATE_VERSION,
        "managed_by": MANAGED_BY,
        "container_name": container_name,
        "container_id": container_id,
        "image": image,
        "port": port,
        "endpoint": endpoint,
        "model": model,
        "served_model_name": served_model_name,
        "backend_name": backend_name,
        "slurm_job_id": os.environ.get(ENV_SLURM_JOB_ID),
        "slurm_job_nodelist": os.environ.get(ENV_SLURM_JOB_NODELIST),
        "started_at": time.time(),
        "command": command,
    }
    _container_state_file(port).write_text(
        json.dumps(state, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )


def _read_container_state(port: int) -> dict[str, Any] | None:
    try:
        state = json.loads(_container_state_file(port).read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return None
    return state if isinstance(state, dict) else None


def _remove_container_state(port: int) -> None:
    try:
        _container_state_file(port).unlink()
    except FileNotFoundError:
        pass
    except OSError as exc:
        _print(f"Could not remove {_container_state_file(port)}: {exc}")


def _port_pids(port: int) -> list[int]:
    if not shutil.which(LSOF):
        return []
    result = _capture([LSOF, f"-tiTCP:{port}", "-sTCP:LISTEN"])
    if result.returncode != 0:
        return []
    pids: list[int] = []
    for line in result.stdout.splitlines():
        try:
            pids.append(int(line.strip()))
        except ValueError:
            pass
    return sorted(set(pids))


def _port_is_available(host: str, port: int) -> bool:
    probe_host = host if host not in ("", ANY_HOST) else LOCALHOST
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        try:
            sock.bind((probe_host, port))
        except OSError:
            return False
    return True


def _port_owner_hint(port: int) -> str:
    pids = _port_pids(port)
    if pids:
        return f"PID(s): {', '.join(map(str, pids))}"
    if not shutil.which(LSOF):
        return f"unknown owner; install {LSOF} for PID attribution"
    return "unknown owner"


def _http_json(
    url: str,
    *,
    method: str | HttpMethod = HttpMethod.GET,
    body: dict[str, Any] | None = None,
    api_key: str | None = None,
) -> Any:
    data = None
    headers = {HttpHeader.CONTENT_TYPE.value: MediaType.JSON.value}
    if api_key:
        headers[HttpHeader.AUTHORIZATION.value] = f"{AuthScheme.BEARER.value} {api_key}"
    if body is not None:
        data = json.dumps(body).encode("utf-8")
    request = urllib.request.Request(
        url,
        data=data,
        headers=headers,
        method=method.value if isinstance(method, HttpMethod) else method,
    )
    with urllib.request.urlopen(request, timeout=DEFAULT_REQUEST_TIMEOUT_SECONDS) as response:
        raw = response.read().decode("utf-8")
    return json.loads(raw) if raw else None


def _server_ready(endpoint: str, *, api_key: str | None = None) -> bool:
    try:
        _http_json(f"{endpoint.rstrip('/')}/{MODELS_PATH}", api_key=api_key)
        return True
    except (urllib.error.URLError, TimeoutError, json.JSONDecodeError):
        return False


def _load_apxm_config() -> dict[str, Any]:
    try:
        with APXM_CONFIG.open("rb") as handle:
            return tomllib.load(handle)
    except FileNotFoundError:
        return {}
    except tomllib.TOMLDecodeError as exc:
        _print(f"Could not parse {APXM_CONFIG}: {exc}")
        return {}


def _backend_config(name: str) -> dict[str, Any] | None:
    for backend in _load_apxm_config().get("backends", []):
        if backend.get("name") == name:
            return backend
    return None


def _backend_model_exists(backend_name: str, model_id: str) -> bool:
    backend = _backend_config(backend_name)
    if not backend:
        return False
    return any(model.get("id") == model_id for model in backend.get("models", []))


def _model_ids(models_response: Any) -> list[str]:
    if not isinstance(models_response, dict):
        return []
    data = models_response.get("data")
    if not isinstance(data, list):
        return []
    ids: list[str] = []
    for item in data:
        if isinstance(item, dict) and isinstance(item.get("id"), str):
            ids.append(item["id"])
    return ids


def _verify_enable_target(endpoint: str, model: str, *, api_key: str | None = None) -> bool:
    base = endpoint.rstrip("/")
    try:
        models = _http_json(f"{base}/{MODELS_PATH}", api_key=api_key)
        ids = _model_ids(models)
        if model not in ids:
            _print(
                f"Served model {model!r} was not reported by {base}/{MODELS_PATH}. "
                f"Reported models: {', '.join(ids) if ids else '<none>'}"
            )
            return False
        _http_json(f"{base}/{APXM_GRAPHS_PATH}/{PROBE_GRAPH_ID}", api_key=api_key)
        scheduler = _http_json(f"{base}/{APXM_SCHEDULER_PATH}", api_key=api_key)
        scheduler_policy = scheduler.get("policy") if isinstance(scheduler, dict) else None
        if scheduler_policy != SchedulingPolicy.PRIORITY.value:
            _print(
                f"APXM scheduler policy is {scheduler_policy!r}; "
                f"expected {SchedulingPolicy.PRIORITY.value!r}."
            )
            return False
    except (urllib.error.URLError, TimeoutError, json.JSONDecodeError) as exc:
        _print(f"Enable preflight failed for {base}: {exc}")
        return False
    return True


def _build_container_vllm_args(args: argparse.Namespace, extra_args: list[str]) -> list[str]:
    served_model_name = args.served_model_name or args.model
    cmd = [
        "--model",
        args.model,
        "--served-model-name",
        served_model_name,
        "--host",
        ANY_HOST,
        "--port",
        str(args.port),
    ]

    if args.dtype:
        cmd.extend(["--dtype", args.dtype])
    if args.max_model_len is not None:
        cmd.extend(["--max-model-len", str(args.max_model_len)])
    if args.tensor_parallel_size is not None:
        cmd.extend(["--tensor-parallel-size", str(args.tensor_parallel_size)])
    if args.gpu_memory_utilization is not None:
        cmd.extend(["--gpu-memory-utilization", str(args.gpu_memory_utilization)])
    if args.download_dir:
        cmd.extend(["--download-dir", args.download_dir])
    if args.enable_auto_tool_choice:
        cmd.append("--enable-auto-tool-choice")
    if args.tool_call_parser:
        cmd.extend(["--tool-call-parser", args.tool_call_parser])
    if args.reasoning_parser:
        cmd.extend([VllmServeFlag.REASONING_PARSER.value, args.reasoning_parser])
    if args.default_chat_template_kwargs:
        try:
            parsed_kwargs = json.loads(args.default_chat_template_kwargs)
        except json.JSONDecodeError as exc:
            raise SystemExit(
                f"--default-chat-template-kwargs must be a JSON object: {exc}"
            ) from exc
        if not isinstance(parsed_kwargs, dict):
            raise SystemExit("--default-chat-template-kwargs must be a JSON object")
        cmd.extend(
            [
                VllmServeFlag.DEFAULT_CHAT_TEMPLATE_KWARGS.value,
                args.default_chat_template_kwargs,
            ]
        )
    if args.enable_prompt_tokens_details:
        cmd.append(VllmServeFlag.ENABLE_PROMPT_TOKENS_DETAILS.value)
    if args.enable_force_include_usage:
        cmd.append(VllmServeFlag.ENABLE_FORCE_INCLUDE_USAGE.value)
    if args.enable_prefix_caching:
        cmd.append(VllmServeFlag.ENABLE_PREFIX_CACHING.value)
    if args.scheduling_policy:
        cmd.extend([VllmServeFlag.SCHEDULING_POLICY.value, args.scheduling_policy])
    if args.trust_remote_code:
        cmd.append("--trust-remote-code")
    cmd.extend(extra_args)
    return cmd


def _extend_container_env(cmd: list[str], env_specs: list[str]) -> bool:
    for spec in env_specs:
        if not spec:
            continue
        if "=" in spec:
            key = spec.split("=", 1)[0]
            if not key:
                _print(f"Invalid --container-env value {spec!r}: missing variable name")
                return False
            cmd.extend([DockerFlag.ENV.value, spec])
            continue
        if spec not in os.environ:
            _print(f"Invalid --container-env value {spec!r}: host environment variable is not set")
            return False
        cmd.extend([DockerFlag.ENV.value, spec])
    return True


def _require_api_key_for_public_bind(args: argparse.Namespace) -> None:
    """Refuse to start vLLM bound to a wildcard host without an API key.

    A wildcard bind without auth would expose the container on shared
    hosts; surface the missing config as a hard error at startup rather
    than silently leaving the port open.
    """
    if args.host in {ANY_HOST, ANY_HOST_V6} and not _api_key(args):
        raise SystemExit(
            f"refusing to bind vLLM on wildcard host {args.host!r} without an "
            f"API key. Set {ENV_VLLM_API_KEY} or pass --api-key (or bind "
            f"loopback with --host {HostAddress.LOOPBACK.value})."
        )


def help_cmd(args: argparse.Namespace) -> int:
    cmd = [sys.executable, str(Path(__file__).resolve())]
    if args.topic:
        cmd.append(args.topic)
    cmd.append("--help")
    return _run(cmd, cwd=REPO_ROOT)


def doctor_cmd(args: argparse.Namespace) -> int:
    errors = 0
    print("mode=canonical-docker")
    print(f"repo_root={REPO_ROOT}")
    print(f"external_vllm={VLLM_DIR}")
    print(f"apxm_commit={_git_stdout(REPO_ROOT, 'rev-parse', 'HEAD') or '<unknown>'}")
    print(f"apxm_dirty={str(_git_dirty(REPO_ROOT)).lower()}")

    if not _verify_fork_source(verbose=True):
        errors += 1

    docker_path = _tool_path(DOCKER)
    print(f"docker={docker_path}")
    docker_ok = _docker_available()
    print(f"docker_daemon_ready={str(docker_ok).lower()}")
    if not docker_ok:
        errors += 1
    buildx_ok = _docker_buildx_available()
    print(f"docker_buildx_ready={str(buildx_ok).lower()}")
    if not buildx_ok:
        errors += 1

    slurm_tools = {tool: _tool_path(tool) for tool in (SINFO, SQUEUE, SBATCH, SRUN)}
    print(
        "slurm_tools="
        + ",".join(f"{tool}:{'yes' if path else 'no'}" for tool, path in slurm_tools.items())
    )
    print(f"slurm_job_id={os.environ.get(ENV_SLURM_JOB_ID, '')}")
    print(f"slurm_job_nodelist={os.environ.get(ENV_SLURM_JOB_NODELIST, '')}")

    created = materialize_data_config(REPO_ROOT)
    if created:
        print("info: wrote .apxm/config.toml from config.example.toml (edit it for non-default paths)")
    data_layout = resolve_data_layout(REPO_ROOT)
    print()
    print(format_data_layout(data_layout, repo_root_label=str(REPO_ROOT)))

    if args.port is not None:
        pids = _port_pids(args.port)
        print(f"port_{args.port}_pids={','.join(map(str, pids)) if pids else ''}")

    return 1 if errors else 0


def probe_cmd(args: argparse.Namespace) -> int:
    base = _endpoint_for_args(args).rstrip("/")
    api_key = _api_key(args)
    graph_id = args.graph_id or _temporary_id(TEMP_GRAPH_ID_PREFIX)
    execution_id = _temporary_id(TEMP_EXECUTION_ID_PREFIX)
    registered = False
    try:
        models = _http_json(f"{base}/{MODELS_PATH}", api_key=api_key)
        print(json.dumps({"models": models}, indent=2, sort_keys=True))
        status = _http_json(f"{base}/{APXM_GRAPHS_PATH}/{PROBE_GRAPH_ID}", api_key=api_key)
        print(json.dumps({"apxm_probe": status}, indent=2, sort_keys=True))
        scheduler = _http_json(f"{base}/{APXM_SCHEDULER_PATH}", api_key=api_key)
        print(json.dumps({"apxm_scheduler": scheduler}, indent=2, sort_keys=True))
        scheduler_policy = scheduler.get("policy") if isinstance(scheduler, dict) else None
        if scheduler_policy != SchedulingPolicy.PRIORITY.value:
            _print(
                f"APXM scheduler policy is {scheduler_policy!r}; "
                f"expected {SchedulingPolicy.PRIORITY.value!r}."
            )
            return 1

        registration = _http_json(
            f"{base}/{APXM_GRAPH_REGISTER_PATH}",
            method=HttpMethod.POST,
            api_key=api_key,
            body={
                "graph_id": graph_id,
                "execution_id": execution_id,
                "nodes": [
                    {
                        "node_id": 1,
                        "node_name": TEMP_NODE_NAME,
                        "downstream_nodes": [],
                    }
                ],
            },
        )
        registered = True
        graph_status = _http_json(f"{base}/{APXM_GRAPHS_PATH}/{graph_id}", api_key=api_key)
        release = _http_json(
            f"{base}/{APXM_GRAPHS_PATH}/{graph_id}",
            method=HttpMethod.DELETE,
            api_key=api_key,
        )
        registered = False
        print(
            json.dumps(
                {
                    "register": registration,
                    "status": graph_status,
                    "release": release,
                },
                indent=2,
                sort_keys=True,
            )
        )
    except (urllib.error.URLError, TimeoutError, json.JSONDecodeError) as exc:
        _print(f"Probe failed for {base}: {exc}")
        return 1
    finally:
        if registered:
            try:
                _http_json(
                    f"{base}/{APXM_GRAPHS_PATH}/{graph_id}",
                    method=HttpMethod.DELETE,
                    api_key=api_key,
                )
            except (urllib.error.URLError, TimeoutError, json.JSONDecodeError) as exc:
                _print(f"Could not release temporary graph {graph_id}: {exc}")
    return 0


def enable_cmd(args: argparse.Namespace) -> int:
    model = args.model
    endpoint = _endpoint_for_args(args)
    api_key = _api_key(args)
    api_key_config = _api_key_config_reference(args)
    if api_key and not api_key_config:
        _print(
            f"Refusing to persist a literal API key. Set {ENV_VLLM_API_KEY}, "
            "or pass --api-key-env <ENV_VAR>, then rerun enable."
        )
        return 1
    add = [
        DekkToken.DEKK.value,
        DekkToken.APXM.value,
        DekkToken.BACKEND.value,
        DekkToken.ADD.value,
        args.backend_name,
        "--type",
        BackendType.ON_PREM.value,
        "--protocol",
        BackendProtocol.VLLM.value,
        "--endpoint",
        endpoint,
    ]
    if api_key_config:
        add.extend(["--api-key", api_key_config])
    add_model = [
        DekkToken.DEKK.value,
        DekkToken.APXM.value,
        DekkToken.BACKEND.value,
        DekkToken.ADD_MODEL.value,
        args.backend_name,
        model,
    ]
    for alias in arg_value(args, ArgName.ALIAS, []):
        add_model.extend(["--alias", alias])
    test = [
        DekkToken.DEKK.value,
        DekkToken.APXM.value,
        DekkToken.BACKEND.value,
        DekkToken.TEST.value,
        args.backend_name,
    ]
    if not _verify_enable_target(endpoint, model, api_key=api_key):
        return 1

    existing = _backend_config(args.backend_name)
    if existing:
        existing_endpoint = str(existing.get("endpoint", ""))
        if _normalize_endpoint(existing_endpoint) != _normalize_endpoint(endpoint):
            _print(
                f"Backend {args.backend_name} already exists with endpoint "
                f"{existing_endpoint}; refusing to register model against {endpoint}."
            )
            _print(f"Remove or update {args.backend_name}, or choose another --backend-name.")
            return 1
        # Endpoint matches — but `enable` is a registration entry point, not
        # a reconciler. Reconciliation belongs to `zoo apply`; refuse to
        # silently no-op on an existing registration.
        _print(
            f"Backend {args.backend_name} is already registered against "
            f"endpoint {existing_endpoint}. `enable` does not reconcile — "
            f"either delete the backend (`dekk apxm backend delete "
            f"{args.backend_name}`) and re-enable, or use `zoo apply` to "
            f"manage drift."
        )
        return 1
    if _run(add, cwd=REPO_ROOT) != 0:
        return 1

    if _backend_model_exists(args.backend_name, model):
        _print(
            f"Model {model} is already registered on backend "
            f"{args.backend_name}. Reconciliation belongs to `zoo apply`; "
            f"`enable` refuses to silently no-op on existing model registrations."
        )
        return 1
    if _run(add_model, cwd=REPO_ROOT) != 0:
        return 1
    return _run(test, cwd=REPO_ROOT)


def docker_build_cmd(args: argparse.Namespace) -> int:
    if not _docker_available():
        _print("Docker is not installed or the daemon is not reachable.")
        return 1
    if not _docker_buildx_available():
        _print(
            "Docker BuildKit/buildx is required; refusing to use Docker's "
            "legacy builder. Install the docker-buildx package/plugin."
        )
        return 1
    if not _verify_fork_source(verbose=True):
        _print("Refusing to build: external/vllm does not expose the APXM fork contract.")
        return 1
    if not args.base_image:
        _print("Refusing to build: pass --base-image with the pinned vLLM image tag or digest.")
        return 1
    dockerfile = Path(args.dockerfile)
    if not dockerfile.is_absolute():
        dockerfile = REPO_ROOT / dockerfile
    if not dockerfile.exists():
        _print(f"Dockerfile not found: {dockerfile}")
        return 1

    image = args.image
    if not image:
        apxm_commit = _capture([GIT, "-C", str(REPO_ROOT), "rev-parse", "--short", "HEAD"]).stdout.strip()
        vllm_commit = _capture([GIT, "-C", str(VLLM_DIR), "rev-parse", "--short", "HEAD"]).stdout.strip()
        image = f"apxm-vllm-runtime:{apxm_commit or 'apxm'}-{vllm_commit or 'vllm'}"

    cmd = [
        DOCKER,
        DockerCommand.BUILDX.value,
        DockerBuildxCommand.BUILD.value,
        DockerFlag.LOAD.value,
        DockerFlag.FILE.value,
        str(dockerfile),
        DockerFlag.TAG.value,
        image,
        DockerFlag.LABEL.value,
        f"org.opencontainers.image.revision={_capture([GIT, '-C', str(REPO_ROOT), 'rev-parse', 'HEAD']).stdout.strip()}",
        DockerFlag.LABEL.value,
        f"apxm.vllm.commit={_capture([GIT, '-C', str(VLLM_DIR), 'rev-parse', 'HEAD']).stdout.strip()}",
        DockerFlag.LABEL.value,
        f"apxm.repo.dirty={str(_git_dirty(REPO_ROOT)).lower()}",
        DockerFlag.LABEL.value,
        f"apxm.vllm.dirty={str(_git_dirty(VLLM_DIR)).lower()}",
    ]
    cmd.extend([DockerFlag.BUILD_ARG.value, f"BASE_IMAGE={args.base_image}"])
    cmd.append(str(REPO_ROOT))

    _print(f"Building APXM-vLLM image: {image}")
    rc = _run(cmd, cwd=REPO_ROOT)
    if rc != 0:
        return rc
    inspect = _capture(
        enum_values(
            [
                ToolName.DOCKER,
                DockerCommand.IMAGE,
                DockerCommand.INSPECT,
                image,
                DockerFlag.FORMAT,
                DockerValue.IMAGE_ID_FORMAT,
            ]
        )
    )
    if inspect.returncode == 0 and inspect.stdout.strip():
        print(f"image={image}")
        print(f"image_id={inspect.stdout.strip()}")
    return 0


def _image_archive_stem(image: str) -> str:
    slug = re.sub(r"[^A-Za-z0-9_.-]+", "_", image).strip("._-")
    if not slug:
        slug = "image"
    digest = hashlib.sha256(image.encode("utf-8")).hexdigest()[:12]
    return f"{slug}-{digest}"


def _default_image_archive(image: str) -> Path:
    return IMAGE_STORE_DIR / f"{_image_archive_stem(image)}.docker.tar"


def _manifest_path_for_archive(archive: Path) -> Path:
    return archive.with_suffix(f"{archive.suffix}.json")


def _docker_image_metadata(image: str) -> dict[str, Any] | None:
    result = _docker_capture(
        enum_values(
            [
                DockerCommand.IMAGE,
                DockerCommand.INSPECT,
                image,
                DockerFlag.FORMAT,
                DockerValue.IMAGE_METADATA_FORMAT,
            ]
        )
    )
    if result.returncode != 0 or not result.stdout.strip():
        return None
    try:
        metadata = json.loads(result.stdout)
    except json.JSONDecodeError:
        return None
    return metadata if isinstance(metadata, dict) else None


def _archive_path_arg(args: argparse.Namespace, image: str) -> Path:
    archive = arg_value(args, ArgName.ARCHIVE)
    return Path(archive).expanduser().resolve() if archive else _default_image_archive(image)


def docker_save_cmd(args: argparse.Namespace) -> int:
    if not _docker_available():
        _print("Docker is not installed or the daemon is not reachable.")
        return 1
    image = args.image
    metadata = _docker_image_metadata(image)
    if metadata is None:
        _print(f"Image is not present in this Docker daemon: {image}")
        return 1

    archive = _archive_path_arg(args, image)
    archive.parent.mkdir(parents=True, exist_ok=True)
    tmp_archive = archive.with_name(f"{archive.name}.tmp")
    tmp_archive.unlink(missing_ok=True)
    result = _docker_capture(
        enum_values([DockerCommand.SAVE, DockerFlag.OUTPUT, str(tmp_archive), image])
    )
    if result.returncode != 0:
        if result.stdout.strip():
            _print(result.stdout.strip())
        if result.stderr.strip():
            _print(result.stderr.strip())
        tmp_archive.unlink(missing_ok=True)
        return result.returncode
    tmp_archive.replace(archive)

    manifest = {
        "schema_version": 1,
        "created_at_unix": time.time(),
        "archive": str(archive),
        "image": image,
        "image_id": metadata.get("Id"),
        "repo_tags": metadata.get("RepoTags"),
        "repo_digests": metadata.get("RepoDigests"),
        "labels": (metadata.get("Config") or {}).get("Labels"),
        "apxm_commit": _git_stdout(REPO_ROOT, "rev-parse", "HEAD"),
        "apxm_dirty": _git_dirty(REPO_ROOT),
        "vllm_commit": _git_stdout(VLLM_DIR, "rev-parse", "HEAD"),
        "vllm_dirty": _git_dirty(VLLM_DIR),
    }
    manifest_path = _manifest_path_for_archive(archive)
    manifest_path.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")
    print(f"image={image}")
    print(f"image_id={metadata.get('Id')}")
    print(f"archive={archive}")
    print(f"manifest={manifest_path}")
    return 0


def docker_load_cmd(args: argparse.Namespace) -> int:
    if not _docker_available():
        _print("Docker is not installed or the daemon is not reachable.")
        return 1
    image = args.image
    archive = _archive_path_arg(args, image)
    if not archive.is_file():
        _print(f"APXM image archive not found: {archive}")
        _print(
            "Create it once from a builder node with: "
            f"dekk apxm vllm docker-save --image {image}"
        )
        return 1

    manifest_path = _manifest_path_for_archive(archive)
    expected_image_id: str | None = None
    if manifest_path.is_file():
        try:
            manifest = json.loads(manifest_path.read_text())
        except json.JSONDecodeError as exc:
            _print(f"Could not parse image archive manifest {manifest_path}: {exc}")
            return 1
        if isinstance(manifest, dict) and isinstance(manifest.get("image_id"), str):
            expected_image_id = manifest["image_id"]

    result = _docker_capture(enum_values([DockerCommand.LOAD, DockerFlag.INPUT, str(archive)]))
    if result.returncode != 0:
        if result.stdout.strip():
            _print(result.stdout.strip())
        if result.stderr.strip():
            _print(result.stderr.strip())
        return result.returncode

    metadata = _docker_image_metadata(image)
    if metadata is None:
        _print(f"Loaded archive, but expected image tag is absent: {image}")
        return 1
    actual_image_id = metadata.get("Id")
    if expected_image_id and actual_image_id != expected_image_id:
        _print(
            f"Loaded image id mismatch for {image}: expected {expected_image_id}, "
            f"got {actual_image_id}"
        )
        return 1

    print(result.stdout.strip())
    print(f"image={image}")
    print(f"image_id={actual_image_id}")
    print(f"archive={archive}")
    return 0


def _resolve_image(args: argparse.Namespace | None = None) -> str:
    """Return the APXM-vLLM image tag, requiring an explicit source.

    Precedence: `--image` flag on the caller's args, else `APXM_VLLM_IMAGE`
    env. No fallback derived from git SHAs — a silently shifting default
    masks image/source-drift bugs, so the image tag must be named outright.
    """
    if args is not None:
        explicit = arg_value(args, ArgName.IMAGE)
        if explicit:
            return str(explicit)
    env_value = os.environ.get(ENV_APXM_VLLM_IMAGE, "").strip()
    if env_value:
        return env_value
    raise SystemExit(
        f"required image not supplied: pass --image, set {ENV_APXM_VLLM_IMAGE}, "
        f"or add `image = '<tag>'` to the manifest entry."
    )


def cache_warm_cmd(args: argparse.Namespace) -> int:
    """Download a model to the shared HF cache without GPU allocation.

    Runs `hf download <model>` inside the APXM-vLLM Docker container with
    no `--gpus` flag, mounting the shared host HF cache so weights persist
    across runs. Idempotent — `hf download` resumes partial downloads via
    the standard HF cache layout.

    The model-zoo deploy pattern is: cache-warm once per model, then `zoo
    apply` boots services that find the weights already on disk.
    """
    if not _docker_available():
        _print("Docker is not installed or the daemon is not reachable.")
        return 1
    image = _resolve_image(args)
    if _docker_image_metadata(image) is None:
        _print(
            f"APXM-vLLM image not loaded: {image}\n"
            f"Load it first with: dekk apxm vllm docker-load --image {image}"
        )
        return 1

    hf_home = effective_hf_home()
    Path(hf_home).mkdir(parents=True, exist_ok=True)

    # The APXM-vLLM image's ENTRYPOINT is the vLLM OpenAI API server.
    # Override it with `hf` (the modern Hugging Face Hub CLI; the legacy
    # `huggingface-cli` is deprecated and no longer works in this image).
    docker_cmd: list[str] = [
        "docker", "run", "--rm",
        DockerFlag.NETWORK.value, DockerValue.HOST_NETWORK.value,
        DockerFlag.ENV.value, f"{ENV_HF_HOME}={ContainerPath.HF_HOME.value}",
        DockerFlag.VOLUME.value, f"{hf_home}:{ContainerPath.HF_HOME.value}",
        "--entrypoint", "hf",
    ]
    if ENV_HF_TOKEN in os.environ:
        docker_cmd.extend([DockerFlag.ENV.value, ENV_HF_TOKEN])
    docker_cmd.append(image)
    docker_cmd.extend(["download", args.model])
    if args.revision:
        docker_cmd.extend(["--revision", args.revision])

    print(f"[cache-warm] image={image}")
    print(f"[cache-warm] model={args.model}")
    print(f"[cache-warm] hf_home_host={hf_home}")
    print(f"[cache-warm] hf_home_container={ContainerPath.HF_HOME.value}")
    print("[cache-warm] starting docker run (CPU-only, no --gpus); resumes from cache if partial")

    return subprocess.run(docker_cmd).returncode


def _service_name(name: str) -> str:
    normalized = re.sub(r"[^A-Za-z0-9_.-]+", "-", name.strip()).strip(".-")
    if not normalized:
        raise SystemExit("service name must contain at least one alphanumeric character")
    return normalized


def _service_state_file(name: str) -> Path:
    return SERVICE_DIR / f"{_service_name(name)}.json"


def _read_service_state(name: str) -> dict[str, Any] | None:
    try:
        state = json.loads(_service_state_file(name).read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return None
    return state if isinstance(state, dict) else None


def _write_service_state(name: str, state: dict[str, Any]) -> Path:
    SERVICE_DIR.mkdir(parents=True, exist_ok=True)
    path = _service_state_file(name)
    path.write_text(json.dumps(state, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return path


def _require_service_state(name: str) -> dict[str, Any] | None:
    state = _read_service_state(name)
    if not state:
        _print(f"APXM-vLLM service state not found: {_service_state_file(name)}")
        return None
    return state


def _service_job_id(state: dict[str, Any]) -> str | None:
    job_id = state.get("job_id")
    return str(job_id) if job_id else None


def _list_service_states() -> list[dict[str, Any]]:
    """Return every recorded service state under SERVICE_DIR, sorted by name."""
    if not SERVICE_DIR.is_dir():
        return []
    states: list[dict[str, Any]] = []
    for path in sorted(SERVICE_DIR.glob("*.json")):
        try:
            payload = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError):
            continue
        if isinstance(payload, dict):
            states.append(payload)
    return states


def _allocated_ports() -> set[int]:
    """Read every recorded service state and return the set of ports in use."""
    ports: set[int] = set()
    for state in _list_service_states():
        port = state.get("port")
        if isinstance(port, int):
            ports.add(port)
        elif isinstance(port, str) and port.isdigit():
            ports.add(int(port))
    return ports


def _allocate_port(*, preferred: int | None = None) -> int:
    """Return the first free port in [PORT_ALLOCATOR_MIN, PORT_ALLOCATOR_MAX].

    The allocator scans recorded service states under SERVICE_DIR — there is no
    runtime port probe. Hard-fails when the range is exhausted instead of
    silently reusing a port, because reuse would let two services point at the
    same Docker container and corrupt service state.
    """
    taken = _allocated_ports()
    if preferred is not None and preferred not in taken:
        return preferred
    for candidate in range(PORT_ALLOCATOR_MIN, PORT_ALLOCATOR_MAX + 1):
        if candidate not in taken:
            return candidate
    raise SystemExit(
        f"port allocator exhausted: every port in "
        f"[{PORT_ALLOCATOR_MIN}, {PORT_ALLOCATOR_MAX}] is already bound to a "
        f"recorded service under {SERVICE_DIR}"
    )


def _squeue_state(job_id: str) -> str:
    if not shutil.which(SQUEUE):
        return "?"
    result = _capture(
        [SQUEUE, "-h", "-j", job_id, "-o", "%T"],
        cwd=REPO_ROOT,
    )
    state = result.stdout.strip().splitlines()
    return state[0] if state else "GONE"


def _start_one_service(
    *,
    name: str,
    model: str,
    port: int,
    image: str,
    backend_name: str,
    served_model_name: str | None = None,
    hf_home: str | None = None,
    max_model_len: int | None = None,
    max_num_seqs: int | None = None,
    scheduling_policy: str | None = None,
    enable_prefix_caching: bool | None = None,
    startup_timeout: float | None = None,
    gpus: str | None = None,
    tensor_parallel_size: int | None = None,
    reasoning_parser: str | None = None,
    tool_call_parser: str | None = None,
    enable_auto_tool_choice: bool | None = None,
) -> tuple[int, dict[str, Any] | None]:
    """Submit one Slurm-owned APXM-vLLM service. Returns (rc, state).

    Pure helper: callers (CLI `service-start`, `zoo apply`) build the kwargs
    explicitly so there is exactly one entry point that turns a configured
    service into a Slurm submission. The function does not inspect argparse
    Namespaces, so `zoo apply` can drive it from manifest data without
    fabricating one.
    """
    if not shutil.which(SBATCH):
        _print("sbatch is not available on this host.")
        return 1, None
    canonical = _service_name(name)
    served = served_model_name or model
    log_path = LOG_DIR / f"slurm-apxm-vllm-service-{canonical}-%j.out"
    script = REPO_ROOT / "deploy" / "vllm" / "run-vllm.sh"
    if not script.is_file():
        _print(f"Unified vLLM service wrapper not found: {script}")
        return 1, None

    env = dict(os.environ)
    env.update(
        {
            ENV_APXM_VLLM_SERVICE_NAME: canonical,
            ENV_APXM_VLLM_IMAGE: image,
            ENV_MODEL_REF: model,
            ENV_SERVED_MODEL_ID: served,
            ENV_BACKEND_NAME: backend_name,
            ENV_PORT: str(port),
        }
    )
    if hf_home:
        env[ENV_HF_HOME_HOST] = hf_home
    if max_model_len is not None:
        env[ENV_MAX_MODEL_LEN] = str(max_model_len)
    if max_num_seqs is not None:
        env[ENV_MAX_NUM_SEQS] = str(max_num_seqs)
    if scheduling_policy:
        env[ENV_SCHEDULING_POLICY] = str(scheduling_policy)
    if enable_prefix_caching is not None:
        env[ENV_ENABLE_PREFIX_CACHING] = "1" if enable_prefix_caching else "0"
    if startup_timeout is not None:
        env[ENV_STARTUP_TIMEOUT_SECONDS] = str(startup_timeout)
    if gpus:
        env[ENV_GPUS] = gpus
    if tensor_parallel_size is not None:
        env[ENV_TENSOR_PARALLEL_SIZE] = str(tensor_parallel_size)
    if reasoning_parser:
        env[env_name(EnvVar.REASONING_PARSER)] = str(reasoning_parser)
    if tool_call_parser:
        env[env_name(EnvVar.TOOL_CALL_PARSER)] = str(tool_call_parser)
    if enable_auto_tool_choice is not None:
        env[env_name(EnvVar.ENABLE_AUTO_TOOL_CHOICE)] = "1" if enable_auto_tool_choice else "0"

    sbatch_cmd = [
        SBATCH,
        "--job-name",
        f"apxm-vllm-{canonical}",
        "--output",
        str(log_path),
        str(script),
    ]

    result = _capture(sbatch_cmd, cwd=REPO_ROOT, env=env)
    if result.stdout.strip():
        print(result.stdout.strip())
    if result.stderr.strip():
        _print(result.stderr.strip())
    if result.returncode != 0:
        return result.returncode, None
    match = re.search(r"Submitted batch job\s+(\d+)", result.stdout)
    if not match:
        _print("Could not parse Slurm job id from sbatch output.")
        return 1, None
    job_id = match.group(1)
    state = {
        "version": STATE_VERSION,
        "managed_by": MANAGED_BY,
        "name": canonical,
        "job_id": job_id,
        "image": image,
        "model": model,
        "served_model_name": served,
        "backend_name": backend_name,
        "port": port,
        "local_endpoint": _endpoint(port),
        "max_model_len": max_model_len,
        "max_num_seqs": max_num_seqs,
        "scheduling_policy": scheduling_policy,
        "enable_prefix_caching": enable_prefix_caching,
        "tensor_parallel_size": tensor_parallel_size,
        "gpus": gpus,
        "log_pattern": str(log_path),
        "submitted_at": time.time(),
    }
    return 0, state


def service_start_cmd(args: argparse.Namespace) -> int:
    """Stub that redirects callers to the manifest-driven deploy path.

    Operators write a manifest entry in `deploy/vllm/zoo.toml` and call
    `dekk apxm vllm zoo-apply`. `_start_one_service` remains as the
    internal entry point that `zoo-apply` invokes.
    """
    del args
    _print(
        "service-start is not supported. "
        "Add a [[deployment]] entry to deploy/vllm/zoo.toml and run "
        "`dekk apxm vllm zoo-apply` instead. See docs/backends/model-zoo.md."
    )
    return 2


ZOO_MANIFEST_SCHEMA_VERSION = 1
ZOO_SNAPSHOT_DIR = LAYOUT.workspace_dir / "deploy"


def _load_zoo_manifest(path: str | Path) -> dict[str, Any]:
    """Parse and validate a zoo manifest TOML file."""
    manifest_path = Path(path)
    if not manifest_path.is_absolute():
        manifest_path = (REPO_ROOT / manifest_path).resolve()
    if not manifest_path.is_file():
        raise SystemExit(
            f"zoo manifest not found: {manifest_path}\n"
            f"  bootstrap with: cp deploy/vllm/zoo.example.toml "
            f"deploy/vllm/zoo.toml && edit"
        )
    with manifest_path.open("rb") as handle:
        data = tomllib.load(handle)
    version = data.get("schema_version", ZOO_MANIFEST_SCHEMA_VERSION)
    if version != ZOO_MANIFEST_SCHEMA_VERSION:
        raise SystemExit(
            f"zoo manifest schema_version={version!r} unsupported "
            f"(controller understands {ZOO_MANIFEST_SCHEMA_VERSION})"
        )
    deployments = data.get("deployment")
    if not isinstance(deployments, list) or not deployments:
        raise SystemExit(
            f"{manifest_path}: must contain at least one [[deployment]] entry"
        )
    defaults = data.get("defaults") or {}
    if not isinstance(defaults, dict):
        raise SystemExit(
            f"{manifest_path}: [defaults] section must be a table, got {type(defaults).__name__}"
        )
    # Merge defaults into every entry that does not override the key. This
    # is done at load time so downstream code sees fully-resolved entries
    # and never has to look at the defaults table again.
    for entry in deployments:
        for key, value in defaults.items():
            entry.setdefault(key, value)
    required_keys = {"name", "model"}
    seen_names: set[str] = set()
    for entry in deployments:
        missing = required_keys - entry.keys()
        if missing:
            raise SystemExit(
                f"{manifest_path}: deployment {entry!r} missing required keys {missing}"
            )
        if entry["name"] in seen_names:
            raise SystemExit(
                f"{manifest_path}: duplicate deployment name {entry['name']!r}"
            )
        seen_names.add(entry["name"])
    data["__path__"] = str(manifest_path)
    return data


def _zoo_replica_names(entry: dict[str, Any]) -> list[str]:
    name = entry["name"]
    replicas = int(entry.get("replicas", 1))
    if replicas < 0:
        raise SystemExit(f"zoo entry {name!r}: replicas must be >= 0")
    if replicas <= 1:
        return [name]
    return [f"{name}-r{i}" for i in range(replicas)]


def _zoo_replica_port(entry: dict[str, Any], replica_index: int) -> int:
    """Resolve the listener port for a single replica.

    Manifest expansion must be deterministic, so the port has to come from
    the manifest. `port_base` covers multi-replica entries, `port` covers
    single-replica entries. Falling back to the runtime port allocator
    would make `zoo expand` non-reproducible — two consecutive runs could
    produce different snapshots — so this raises instead.
    """
    if "port_base" in entry:
        return int(entry["port_base"]) + replica_index
    if "port" in entry and int(entry.get("replicas", 1)) <= 1:
        return int(entry["port"])
    raise SystemExit(
        f"zoo entry {entry['name']!r}: missing 'port' (single replica) or "
        f"'port_base' (replicas > 1). Manifest must specify an explicit "
        f"port so expansion is deterministic."
    )


def _zoo_replica_gpus(entry: dict[str, Any], replica_index: int) -> str | None:
    explicit_groups = entry.get("gpu_groups")
    if explicit_groups is not None:
        try:
            group = explicit_groups[replica_index]
        except IndexError as exc:
            raise SystemExit(
                f"zoo entry {entry['name']!r}: gpu_groups has fewer entries than replicas"
            ) from exc
        return ",".join(str(idx) for idx in group)
    return entry.get("gpus")


def _zoo_expand_entries(manifest: dict[str, Any]) -> list[dict[str, Any]]:
    """Expand manifest deployment entries into one record per service to start."""
    expanded: list[dict[str, Any]] = []
    for entry in manifest["deployment"]:
        replica_names = _zoo_replica_names(entry)
        for idx, replica_name in enumerate(replica_names):
            expanded.append(
                {
                    "name": replica_name,
                    "manifest_name": entry["name"],
                    "replica_index": idx,
                    "replica_count": len(replica_names),
                    "model": entry["model"],
                    # Per-entry `image` override (manifest > env).
                    "image": entry.get("image"),
                    "served_model_name": entry.get("served_model_name", entry["model"]),
                    "backend_name": (
                        entry.get("backend_name") if len(replica_names) == 1
                        else f"{entry.get('backend_name', entry['name'])}-r{idx}"
                    ),
                    "port": _zoo_replica_port(entry, idx),
                    "gpus": _zoo_replica_gpus(entry, idx),
                    "tensor_parallel_size": entry.get("tensor_parallel"),
                    "max_model_len": entry.get("max_model_len"),
                    "max_num_seqs": entry.get("max_num_seqs"),
                    "scheduling_policy": entry.get(
                        "scheduling_policy", SERVICE_DEFAULTS.scheduling_policy
                    ),
                    "enable_prefix_caching": entry.get(
                        "enable_prefix_caching", SERVICE_DEFAULTS.enable_prefix_caching
                    ),
                    "startup_timeout": entry.get(
                        "startup_timeout", SERVICE_DEFAULTS.startup_timeout_seconds
                    ),
                    "weights_gb": entry.get("weights_gb"),
                    # Model-specific feature toggles: NO default. A silent
                    # `reasoning_parser=openai_gptoss` default crashes every
                    # non-gpt-oss model at vLLM startup with a vocab
                    # KeyError. Manifest must set per-deployment.
                    "reasoning_parser": entry.get("reasoning_parser"),
                    "tool_call_parser": entry.get("tool_call_parser"),
                    "enable_auto_tool_choice": entry.get("enable_auto_tool_choice"),
                }
            )
    return expanded


def _zoo_disk_pre_check(manifest: dict[str, Any]) -> None:
    """Refuse to start if the HF cache filesystem has less free space than Σ(weights_gb)*1.2."""
    total_weights_gb = sum(
        float(entry.get("weights_gb") or 0)
        for entry in manifest["deployment"]
    )
    if total_weights_gb <= 0:
        return
    required_gb = total_weights_gb * 1.2
    home = Path.home()
    try:
        usage = shutil.disk_usage(home)
    except OSError as exc:
        raise SystemExit(
            f"could not stat HF cache filesystem at {home}: {exc}"
        ) from exc
    free_gb = usage.free / (1024**3)
    if free_gb < required_gb:
        raise SystemExit(
            f"zoo cache-warm refused: WekaFS free at {home} = {free_gb:.1f} GB; "
            f"required = Σ(weights_gb) * 1.2 = {required_gb:.1f} GB. "
            f"Free up space or coordinate a shared HF cache namespace before retrying."
        )


def zoo_cache_warm_cmd(args: argparse.Namespace) -> int:
    """Warm the HF cache for every model in the manifest (CPU-only, idempotent)."""
    manifest = _load_zoo_manifest(args.manifest)
    _zoo_disk_pre_check(manifest)
    seen: set[str] = set()
    rc = 0
    for entry in manifest["deployment"]:
        model = entry["model"]
        if model in seen:
            continue
        seen.add(model)
        print(f"[zoo cache-warm] {model}")
        # Prefer the manifest entry's image (defaults-merged by
        # _load_zoo_manifest), then fall through to args/env in
        # _resolve_image. No silent factory fallback.
        ns = argparse.Namespace(
            model=model,
            image=entry.get("image") or getattr(args, "image", None),
            hf_home=getattr(args, "hf_home", None),
            revision=entry.get("revision"),
        )
        step_rc = cache_warm_cmd(ns)
        if step_rc != 0:
            _print(f"cache-warm failed for {model} (rc={step_rc})")
            rc = step_rc
    return rc


def _zoo_snapshot_dir() -> Path:
    ts = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
    target = ZOO_SNAPSHOT_DIR / ts
    target.mkdir(parents=True, exist_ok=True)
    return target


def _write_zoo_snapshot(manifest: dict[str, Any], started: list[dict[str, Any]]) -> Path:
    target = _zoo_snapshot_dir()
    payload = {
        "manifest_path": manifest.get("__path__"),
        "manifest": {k: v for k, v in manifest.items() if k != "__path__"},
        "services": started,
        "captured_at": time.time(),
    }
    snapshot = target / "zoo-snapshot.json"
    snapshot.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return snapshot


def zoo_apply_cmd(args: argparse.Namespace) -> int:
    """Reconcile the zoo manifest against currently-recorded services.

    Reconciliation rules:
      - Missing → _start_one_service.
      - Removed → warn only (operator must `zoo scale --replicas 0` or
        `zoo apply --prune` to opt into cancellation, protecting peer jobs).
      - Present → service_status_cmd probe.
    Always emits `.apxm/deploy/<TIMESTAMP>/zoo-snapshot.json` for provenance.
    """
    manifest = _load_zoo_manifest(args.manifest)
    desired = _zoo_expand_entries(manifest)
    desired_names = {entry["name"] for entry in desired}
    existing = {state.get("name"): state for state in _list_service_states()}
    # No `_default_image_tag()` fallback — every manifest entry must either
    # specify `image = '<tag>'` or rely on the operator's APXM_VLLM_IMAGE
    # env. _resolve_image hard-fails if neither is set.

    started: list[dict[str, Any]] = []
    rc = 0
    for entry in desired:
        if entry["name"] in existing:
            print(f"[zoo apply] already running: {entry['name']} (probing)")
            probe_ns = argparse.Namespace(name=entry["name"], probe=False)
            service_status_cmd(probe_ns)
            started.append({"name": entry["name"], "job_id": existing[entry["name"]].get("job_id"), "status": "existing"})
            continue
        print(f"[zoo apply] starting {entry['name']} on port {entry['port']}")
        step_rc, state = _start_one_service(
            name=entry["name"],
            model=entry["model"],
            port=entry["port"],
            image=entry["image"] or _resolve_image(),
            backend_name=entry["backend_name"] or DEFAULT_BACKEND_NAME,
            served_model_name=entry["served_model_name"],
            hf_home=effective_hf_home(),
            max_model_len=entry["max_model_len"],
            max_num_seqs=entry["max_num_seqs"],
            scheduling_policy=entry["scheduling_policy"],
            enable_prefix_caching=entry["enable_prefix_caching"],
            startup_timeout=entry["startup_timeout"],
            gpus=entry["gpus"],
            tensor_parallel_size=entry["tensor_parallel_size"],
            reasoning_parser=entry.get("reasoning_parser"),
            tool_call_parser=entry.get("tool_call_parser"),
            enable_auto_tool_choice=entry.get("enable_auto_tool_choice"),
        )
        if step_rc != 0 or state is None:
            _print(f"[zoo apply] failed to start {entry['name']} (rc={step_rc})")
            rc = step_rc or 1
            continue
        _write_service_state(entry["name"], state)
        started.append({"name": entry["name"], "job_id": state["job_id"], "status": "started"})

    for stale_name in sorted(existing.keys() - desired_names):
        if not getattr(args, ArgName.PRUNE.value, False):
            _print(
                f"[zoo apply] WARN: service {stale_name!r} is recorded but not in "
                f"manifest. Re-run with --prune to cancel (peer-protection: off by default)."
            )
            continue
        _print(f"[zoo apply] pruning {stale_name}")
        stop_ns = argparse.Namespace(name=stale_name, remove_state=True)
        service_stop_cmd(stop_ns)

    snapshot = _write_zoo_snapshot(manifest, started)
    print(f"snapshot={snapshot}")
    return rc


def zoo_status_cmd(args: argparse.Namespace) -> int:
    manifest = _load_zoo_manifest(args.manifest)
    desired = _zoo_expand_entries(manifest)
    for entry in desired:
        print(f"--- {entry['name']} ---")
        ns = argparse.Namespace(name=entry["name"], probe=False)
        service_status_cmd(ns)
    return 0


def zoo_scale_cmd(args: argparse.Namespace) -> int:
    manifest = _load_zoo_manifest(args.manifest)
    name = args.name
    replicas = int(args.replicas)
    entry = next((d for d in manifest["deployment"] if d["name"] == name), None)
    if entry is None:
        _print(f"zoo entry {name!r} not found in {manifest['__path__']}")
        return 1
    entry["replicas"] = replicas
    desired = _zoo_expand_entries({"deployment": [entry]})
    desired_names = {e["name"] for e in desired}
    existing = {
        state["name"]: state
        for state in _list_service_states()
        if isinstance(state.get("name"), str)
        and (state["name"] == name or state["name"].startswith(f"{name}-r"))
    }
    rc = 0
    for stale_name in sorted(existing.keys() - desired_names):
        _print(f"[zoo scale] stopping {stale_name}")
        stop_ns = argparse.Namespace(name=stale_name, remove_state=True)
        service_stop_cmd(stop_ns)
    # No `_default_image_tag()` fallback — every manifest entry must either
    # specify `image = '<tag>'` or rely on the operator's APXM_VLLM_IMAGE
    # env. _resolve_image hard-fails if neither is set.
    for e in desired:
        if e["name"] in existing:
            continue
        step_rc, state = _start_one_service(
            name=e["name"],
            model=e["model"],
            port=e["port"],
            image=entry["image"] or _resolve_image(),
            backend_name=e["backend_name"] or DEFAULT_BACKEND_NAME,
            served_model_name=e["served_model_name"],
            hf_home=effective_hf_home(),
            max_model_len=e["max_model_len"],
            max_num_seqs=e["max_num_seqs"],
            scheduling_policy=e["scheduling_policy"],
            enable_prefix_caching=e["enable_prefix_caching"],
            startup_timeout=e["startup_timeout"],
            gpus=e["gpus"],
            tensor_parallel_size=e["tensor_parallel_size"],
            reasoning_parser=e.get("reasoning_parser"),
            tool_call_parser=e.get("tool_call_parser"),
            enable_auto_tool_choice=e.get("enable_auto_tool_choice"),
        )
        if step_rc != 0 or state is None:
            rc = step_rc or 1
            continue
        _write_service_state(e["name"], state)
    return rc


def zoo_logs_cmd(args: argparse.Namespace) -> int:
    manifest = _load_zoo_manifest(args.manifest)
    desired_names = {e["name"] for e in _zoo_expand_entries(manifest)}
    for state in _list_service_states():
        name = state.get("name")
        if name not in desired_names:
            continue
        job_id = _service_job_id(state)
        if not job_id:
            continue
        print(f"--- {name} (job_id={job_id}) ---")
        port = state.get("port")
        if not port:
            _print(f"[zoo logs] skipping {name}: state has no recorded port")
            continue
        _run(
            [SRUN, "--jobid", job_id, "--overlap", "docker", "logs", "--tail=80",
             f"apxm-vllm-{job_id}-{port}"],
            cwd=REPO_ROOT,
        )
    return 0


def zoo_dispatch(args: argparse.Namespace, cmd: VllmCommand) -> int:
    handlers = {
        VllmCommand.ZOO_APPLY: zoo_apply_cmd,
        VllmCommand.ZOO_STATUS: zoo_status_cmd,
        VllmCommand.ZOO_SCALE: zoo_scale_cmd,
        VllmCommand.ZOO_CACHE_WARM: zoo_cache_warm_cmd,
        VllmCommand.ZOO_LOGS: zoo_logs_cmd,
    }
    handler = handlers.get(cmd)
    if handler is None:
        raise SystemExit(f"unknown zoo command: {cmd.value}")
    return handler(args)


def service_list_cmd(args: argparse.Namespace) -> int:
    """Print every recorded service alongside its current Slurm state."""
    states = _list_service_states()
    if not states:
        print(f"(no services recorded under {SERVICE_DIR})")
        return 0
    rows = []
    for state in states:
        job_id = _service_job_id(state) or "-"
        slurm_state = _squeue_state(job_id) if job_id != "-" else "-"
        rows.append(
            {
                "name": state.get("name", "?"),
                "port": str(state.get("port", "?")),
                "job_id": job_id,
                "slurm": slurm_state,
                "model": state.get("model", "?"),
                "backend": state.get("backend_name", "?"),
            }
        )
    widths = {key: max(len(key), max(len(row[key]) for row in rows)) for key in rows[0]}
    header = "  ".join(key.upper().ljust(widths[key]) for key in rows[0])
    print(header)
    for row in rows:
        print("  ".join(row[key].ljust(widths[key]) for key in rows[0]))
    return 0


def service_status_cmd(args: argparse.Namespace) -> int:
    state = _require_service_state(args.name)
    if not state:
        return 1
    job_id = _service_job_id(state)
    print(json.dumps(state, indent=2, sort_keys=True))
    if job_id and shutil.which(SQUEUE):
        result = _capture(
            [SQUEUE, "-j", job_id, "-o", "%.18i %.9P %.30j %.8u %.2t %.12M %.12l %.20R"],
            cwd=REPO_ROOT,
        )
        if result.stdout.strip():
            print(result.stdout.strip())
        if result.stderr.strip():
            _print(result.stderr.strip())
    if args.probe and job_id:
        port = state.get("port")
        if not port:
            _print(f"[service-status] cannot probe {args.name}: state has no recorded port")
            return 1
        return _run(
            [
                SRUN,
                "--jobid",
                job_id,
                "--overlap",
                sys.executable,
                str(Path(__file__).resolve()),
                VllmCommand.PROBE.value,
                "--port",
                str(port),
            ],
            cwd=REPO_ROOT,
        )
    return 0


def service_exec_cmd(args: argparse.Namespace, extra_args: list[str]) -> int:
    state = _require_service_state(args.name)
    if not state:
        return 1
    job_id = _service_job_id(state)
    if not job_id:
        _print(f"Service {args.name} has no Slurm job id.")
        return 1
    command = list(args.service_command or extra_args)
    if command and command[0] == "--":
        command = command[1:]
    if not command:
        _print("service-exec requires a command after --")
        return 1
    env = dict(os.environ)
    service_env = {
        ENV_APXM_VLLM_IMAGE: state.get("image"),
        ENV_MODEL_REF: state.get("model"),
        ENV_SERVED_MODEL_ID: state.get("served_model_name"),
        ENV_BACKEND_NAME: state.get("backend_name"),
        ENV_PORT: state.get("port"),
        ENV_MAX_MODEL_LEN: state.get("max_model_len"),
        ENV_MAX_NUM_SEQS: state.get("max_num_seqs"),
        ENV_SCHEDULING_POLICY: state.get("scheduling_policy"),
        ENV_ENABLE_PREFIX_CACHING: (
            "1" if state.get("enable_prefix_caching") is True else
            "0" if state.get("enable_prefix_caching") is False else None
        ),
    }
    for key, value in service_env.items():
        if value not in (None, ""):
            env[key] = str(value)
    return _run([SRUN, "--jobid", job_id, "--overlap", *command], cwd=REPO_ROOT, env=env)


def service_stop_cmd(args: argparse.Namespace) -> int:
    state = _require_service_state(args.name)
    if not state:
        return 1
    job_id = _service_job_id(state)
    if not job_id:
        _print(f"Service {args.name} has no Slurm job id.")
        return 1
    if not shutil.which(SCANCEL):
        _print("scancel is not available on this host.")
        return 1
    rc = _run([SCANCEL, job_id], cwd=REPO_ROOT)
    if rc == 0 and args.remove_state:
        try:
            _service_state_file(args.name).unlink()
        except FileNotFoundError:
            pass
    return rc


def docker_start_cmd(args: argparse.Namespace, extra_args: list[str]) -> int:
    if not _docker_available():
        _print("Docker is not installed or the daemon is not reachable.")
        return 1
    if not args.image:
        _print("--image is required for docker-start")
        return 1
    if args.port is None:
        _print("--port is required for docker-start")
        return 1

    container_name = args.container_name
    if not container_name:
        _print(
            "--container-name is required for docker-start. "
            "The unified wrapper deploy/vllm/run-vllm.sh derives it from "
            "the service name."
        )
        return 1
    if _docker_container_running(container_name):
        _print(f"Container {container_name} is already running.")
        return 1
    if not _port_is_available(LOCALHOST, args.port):
        _print(f"Port {args.port} is already in use by {_port_owner_hint(args.port)}")
        return 1

    LOG_DIR.mkdir(parents=True, exist_ok=True)
    served_model_name = args.served_model_name or args.model
    endpoint = _endpoint_for_args(args)
    hf_home = _hf_home(args)
    cmd = [
        DOCKER,
        DockerCommand.RUN.value,
        DockerFlag.DETACH.value,
        DockerFlag.NAME.value,
        container_name,
        DockerFlag.NETWORK.value,
        DockerValue.HOST_NETWORK.value,
        DockerFlag.IPC_HOST.value,
        f"{DockerFlag.DEVICE.value}=/dev/kfd",
        f"{DockerFlag.DEVICE.value}=/dev/dri",
        DockerFlag.GROUP_ADD.value,
        "video",
        DockerFlag.GROUP_ADD.value,
        "render",
        DockerFlag.SECURITY_OPT.value,
        DockerValue.SECCOMP_UNCONFINED.value,
        DockerFlag.LABEL.value,
        f"{DockerLabel.MANAGED_BY.value}={MANAGED_BY}",
        DockerFlag.LABEL.value,
        f"{DockerLabel.BACKEND_NAME.value}={args.backend_name}",
        DockerFlag.LABEL.value,
        f"{DockerLabel.SERVED_MODEL_ID.value}={served_model_name}",
    ]
    slurm_job_id = os.environ.get(ENV_SLURM_JOB_ID, "").strip()
    if slurm_job_id:
        cmd.extend([DockerFlag.LABEL.value, f"{DockerLabel.SLURM_JOB_ID.value}={slurm_job_id}"])
    if hf_home:
        Path(hf_home).mkdir(parents=True, exist_ok=True)
        cmd.extend(
            [
                DockerFlag.ENV.value,
                f"{ENV_HF_HOME}={ContainerPath.HF_HOME.value}",
                DockerFlag.VOLUME.value,
                f"{hf_home}:{ContainerPath.HF_HOME.value}",
            ]
        )
    if args.gpus:
        cmd.extend(
            [
                DockerFlag.ENV.value,
                f"{ENV_HIP_VISIBLE_DEVICES}={args.gpus}",
                DockerFlag.ENV.value,
                f"{ENV_CUDA_VISIBLE_DEVICES}={args.gpus}",
            ]
        )
    if ENV_HF_TOKEN in os.environ:
        cmd.extend([DockerFlag.ENV.value, ENV_HF_TOKEN])
    if _api_key(args):
        cmd.extend([DockerFlag.ENV.value, f"{ENV_VLLM_API_KEY}={_api_key(args)}"])
    if not _extend_container_env(cmd, arg_value(args, ArgName.CONTAINER_ENV, [])):
        return 1

    cmd.append(args.image)
    cmd.extend(_build_container_vllm_args(args, extra_args))

    _require_api_key_for_public_bind(args)
    _print(f"Starting APXM-vLLM container {container_name}")
    _print(f"image={args.image}")
    _print(f"endpoint={endpoint}")
    _print(f"hf_home={hf_home}")
    result = _capture(cmd, cwd=REPO_ROOT)
    if result.returncode != 0:
        if result.stdout.strip():
            _print(result.stdout.strip())
        if result.stderr.strip():
            _print(result.stderr.strip())
        return result.returncode

    container_id = result.stdout.strip()
    _write_container_state(
        port=args.port,
        container_name=container_name,
        container_id=container_id,
        image=args.image,
        endpoint=endpoint,
        model=args.model,
        served_model_name=served_model_name,
        backend_name=args.backend_name,
        command=cmd,
    )
    _print(f"container_id={container_id}")
    _print(f"state={_container_state_file(args.port)}")

    if args.wait:
        deadline = time.time() + args.startup_timeout
        while time.time() < deadline:
            if not _docker_container_running(container_name):
                _print(f"Container {container_name} exited before server became ready.")
                docker_logs_cmd(
                    argparse.Namespace(
                        container_name=container_name,
                        port=args.port,
                        lines=5000,
                        follow=False,
                    )
                )
                return 1
            if _server_ready(endpoint, api_key=_api_key(args)):
                _print(f"Server responded on local_probe_endpoint={endpoint}")
                if args.enable:
                    probe_args = argparse.Namespace(
                        endpoint=endpoint,
                        port=args.port,
                        api_key=arg_value(args, ArgName.API_KEY),
                        graph_id=None,
                    )
                    if probe_cmd(probe_args) != 0:
                        return 1
                    enable_args = argparse.Namespace(
                        model=served_model_name,
                        backend_name=args.backend_name,
                        port=args.port,
                        endpoint=endpoint,
                        api_key=arg_value(args, ArgName.API_KEY),
                        api_key_env=arg_value(args, ArgName.API_KEY_ENV),
                        alias=arg_value(args, ArgName.ALIAS, []),
                    )
                    return enable_cmd(enable_args)
                return 0
            time.sleep(2.0)
        _print(
            f"Timed out waiting for local_probe_endpoint={endpoint}. "
            f"Run: dekk apxm vllm docker-logs --port {args.port}"
        )
        return 1
    return 0


def _container_name_from_args(args: argparse.Namespace) -> str:
    if args.container_name:
        return args.container_name
    state = _read_container_state(args.port)
    if state and isinstance(state.get("container_name"), str):
        return state["container_name"]
    raise SystemExit(
        f"--container-name required (no container state at "
        f"{_container_state_file(args.port)}); the name comes from the "
        f"service state recorded by `zoo-apply`."
    )


def docker_stop_cmd(args: argparse.Namespace) -> int:
    if not _docker_available():
        _print("Docker is not installed or the daemon is not reachable.")
        return 1
    name = _container_name_from_args(args)
    result = _docker_capture(enum_values([DockerCommand.RM, DockerFlag.FORCE, name]))
    if result.returncode != 0:
        if result.stderr.strip():
            _print(result.stderr.strip())
        return result.returncode
    if result.stdout.strip():
        print(result.stdout.strip())
    _remove_container_state(args.port)
    return 0


def docker_status_cmd(args: argparse.Namespace) -> int:
    if not _docker_available():
        _print("Docker is not installed or the daemon is not reachable.")
        return 1
    name = _container_name_from_args(args)
    result = _docker_capture(
        enum_values([DockerCommand.INSPECT, DockerFlag.FORMAT, DockerValue.STATUS_FORMAT, name])
    )
    if result.returncode == 0 and result.stdout.strip():
        print(result.stdout.strip().lstrip("/"))
    else:
        print(f"name={name} status=not_found")
    state = _read_container_state(args.port)
    if state:
        print(f"state={_container_state_file(args.port)}")
        print(f"endpoint={state.get('endpoint')}")
        print(f"model={state.get('model')}")
        print(f"served_model_name={state.get('served_model_name')}")
        print(f"backend_name={state.get('backend_name')}")
    return 0


def docker_logs_cmd(args: argparse.Namespace) -> int:
    if not _docker_available():
        _print("Docker is not installed or the daemon is not reachable.")
        return 1
    name = _container_name_from_args(args)
    cmd = [DOCKER, DockerCommand.LOGS.value, DockerFlag.TAIL.value, str(args.lines)]
    if args.follow:
        cmd.append(DockerFlag.FOLLOW.value)
    cmd.append(name)
    return _run(cmd, cwd=REPO_ROOT)


def _add_model_args(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("model", help="vLLM model id or local model path")
    parser.add_argument("--backend-name", default=DEFAULT_BACKEND_NAME, help="APXM backend name")
    parser.add_argument(
        "--host",
        default=DEFAULT_HOST,
        help="vLLM bind host; does not change the APXM registration endpoint",
    )
    parser.add_argument("--port", type=int, default=None, help="Bind port for vLLM")
    parser.add_argument(
        "--hf-home",
        help=(
            "Cache root for Hugging Face-backed model refs; optional for local "
            f"paths (or set {ENV_APXM_VLLM_HF_HOME}/{ENV_HF_HOME})"
        ),
    )
    parser.add_argument("--served-model-name", help="Override the model id exposed by the server")
    parser.add_argument(
        "--endpoint",
        help="Client base URL, including /v1, used for APXM registration",
    )
    parser.add_argument(
        "--gpus",
        help="GPU list, applied to both HIP_VISIBLE_DEVICES and CUDA_VISIBLE_DEVICES",
    )
    parser.add_argument("--dtype", help="Model dtype passed through to vLLM")
    parser.add_argument("--max-model-len", type=int, help="Maximum model context length")
    parser.add_argument("--tensor-parallel-size", type=int, help="Tensor parallel size")
    parser.add_argument(
        "--gpu-memory-utilization",
        type=float,
        help="GPU memory utilization fraction",
    )
    parser.add_argument("--download-dir", help="Model download/cache directory")
    parser.add_argument(
        "--api-key",
        help=f"API key required by the vLLM server; start/serve pass it via {ENV_VLLM_API_KEY}",
    )
    parser.add_argument("--tool-call-parser", help="Enable a specific tool-call parser")
    parser.add_argument(
        "--enable-auto-tool-choice",
        action="store_true",
        help="Enable auto tool choice support",
    )
    parser.add_argument(
        "--reasoning-parser",
        dest=ArgName.REASONING_PARSER.value,
        help="Enable a model-specific vLLM reasoning parser",
    )
    parser.add_argument(
        "--default-chat-template-kwargs",
        dest=ArgName.DEFAULT_CHAT_TEMPLATE_KWARGS.value,
        help="JSON object forwarded to vLLM as default chat-template kwargs",
    )
    parser.add_argument(
        "--enable-prompt-tokens-details",
        dest=ArgName.ENABLE_PROMPT_TOKENS_DETAILS.value,
        action=argparse.BooleanOptionalAction,
        default=DEFAULTS.enable_prompt_tokens_details,
        help=(
            "Emit usage.prompt_tokens_details (cached_tokens) on responses. "
            "APXM defaults this on so the OpenAI parser can populate "
            "cached_input_tokens; pass --no-enable-prompt-tokens-details to opt out."
        ),
    )
    parser.add_argument(
        "--enable-force-include-usage",
        dest=ArgName.ENABLE_FORCE_INCLUDE_USAGE.value,
        action="store_true",
        help="Ask vLLM to include usage on every supported request",
    )
    parser.add_argument(
        "--enable-prefix-caching",
        dest=ArgName.ENABLE_PREFIX_CACHING.value,
        action="store_true",
        help="Enable vLLM prefix caching for graph-aware latency experiments",
    )
    parser.add_argument(
        VllmServeFlag.SCHEDULING_POLICY.value,
        dest=ArgName.SCHEDULING_POLICY.value,
        choices=[p.value for p in SchedulingPolicy],
        default=DEFAULTS.scheduling_policy,
        help=(
            "vLLM scheduler policy. Single-variant under APXM (priority) "
            "so compiler-stamped critical-path hints re-order the waiting "
            "queue. The upstream FCFS branch is no longer selectable."
        ),
    )
    parser.add_argument(
        "--trust-remote-code",
        action="store_true",
        help="Pass --trust-remote-code to vLLM",
    )


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="dekk apxm vllm",
        description=(
            "Operate APXM-vLLM through persistent Slurm services, with Docker "
            "image-store controls as allocation-local primitives."
        ),
    )
    subparsers = parser.add_subparsers(dest=ArgName.COMMAND.value, required=True)

    help_parser = subparsers.add_parser(VllmCommand.HELP.value, help="Show controller or subcommand help")
    help_parser.add_argument("topic", nargs="?", help="Subcommand to describe")
    help_parser.set_defaults(**{ArgName.HANDLER.value: lambda ns, extra: help_cmd(ns)})

    doctor = subparsers.add_parser(VllmCommand.DOCTOR.value, help="Verify APXM-vLLM Docker, fork source, GPU, Slurm, and port readiness")
    doctor.add_argument(
        "--hf-home",
        help=f"HF_HOME value to report/use for Hugging Face-backed refs (or set {ENV_APXM_VLLM_HF_HOME}/{ENV_HF_HOME})",
    )
    doctor.add_argument("--port", type=int, default=None, help="vLLM port")
    doctor.set_defaults(**{ArgName.HANDLER.value: lambda ns, extra: doctor_cmd(ns)})

    docker_build = subparsers.add_parser(
        VllmCommand.DOCKER_BUILD.value,
        help="Build a pinned APXM-vLLM runtime Docker image",
    )
    docker_build.add_argument(
        "--dockerfile",
        dest=ArgName.DOCKERFILE.value,
        default="deploy/vllm/Dockerfile.apxm",
        help="Dockerfile used to build the APXM-vLLM image",
    )
    docker_build.add_argument(
        "--image",
        dest=ArgName.IMAGE.value,
        help="Image tag to create (default: apxm-vllm-runtime:<apxm>-<vllm>)",
    )
    docker_build.add_argument(
        "--base-image",
        dest=ArgName.BASE_IMAGE.value,
        help="Override Dockerfile BASE_IMAGE build arg",
    )
    docker_build.set_defaults(**{ArgName.HANDLER.value: lambda ns, extra: docker_build_cmd(ns)})

    docker_save = subparsers.add_parser(
        VllmCommand.DOCKER_SAVE.value,
        help="Save a built APXM-vLLM image into the APXM shared image store",
    )
    docker_save.add_argument(
        "--image",
        dest=ArgName.IMAGE.value,
        required=True,
        help="Image tag/digest to save",
    )
    docker_save.add_argument(
        "--archive",
        dest=ArgName.ARCHIVE.value,
        help="Archive path (default: .apxm/vllm-images/<image>.docker.tar)",
    )
    docker_save.set_defaults(**{ArgName.HANDLER.value: lambda ns, extra: docker_save_cmd(ns)})

    docker_load = subparsers.add_parser(
        VllmCommand.DOCKER_LOAD.value,
        help="Load an APXM-vLLM image from the APXM shared image store",
    )
    docker_load.add_argument(
        "--image",
        dest=ArgName.IMAGE.value,
        required=True,
        help="Image tag/digest expected after load",
    )
    docker_load.add_argument(
        "--archive",
        dest=ArgName.ARCHIVE.value,
        help="Archive path (default: .apxm/vllm-images/<image>.docker.tar)",
    )
    docker_load.set_defaults(**{ArgName.HANDLER.value: lambda ns, extra: docker_load_cmd(ns)})

    cache_warm = subparsers.add_parser(
        VllmCommand.CACHE_WARM.value,
        help="Download a model to the shared HF cache without GPU allocation",
    )
    cache_warm.add_argument(
        "model",
        help="HF model id or path to download (e.g. openai/gpt-oss-120b)",
    )
    cache_warm.add_argument(
        "--image",
        dest=ArgName.IMAGE.value,
        help="APXM-vLLM image tag/digest (default: APXM_VLLM_IMAGE or apxm-vllm-runtime:<apxm>-<vllm>)",
    )
    cache_warm.add_argument(
        "--hf-home",
        dest=ArgName.HF_HOME.value,
        help=f"Host Hugging Face cache root (or set {ENV_APXM_VLLM_HF_HOME}/{ENV_HF_HOME})",
    )
    cache_warm.add_argument(
        "--revision",
        default=None,
        help="Specific revision/branch/tag to download (default: main)",
    )
    cache_warm.set_defaults(**{ArgName.HANDLER.value: lambda ns, extra: cache_warm_cmd(ns)})

    docker_start = subparsers.add_parser(
        VllmCommand.DOCKER_START.value,
        help="Start vLLM from a Docker image inside an owned allocation",
    )
    _add_model_args(docker_start)
    docker_start.add_argument(
        "--image",
        dest=ArgName.IMAGE.value,
        required=True,
        help="APXM-vLLM image tag/digest",
    )
    docker_start.add_argument(
        "--container-name",
        dest=ArgName.CONTAINER_NAME.value,
        help="Docker container name (required — no factory default)",
    )
    docker_start.add_argument(
        "--wait",
        dest=ArgName.WAIT.value,
        action="store_true",
        default=True,
        help="Wait until /v1/models responds (default)",
    )
    docker_start.add_argument(
        "--no-wait",
        dest=ArgName.WAIT.value,
        action="store_false",
        help="Return after launching the container",
    )
    docker_start.add_argument(
        "--startup-timeout",
        type=float,
        default=DEFAULT_STARTUP_TIMEOUT_SECONDS,
        help="Seconds to wait when --wait is set",
    )
    docker_start.add_argument(
        "--enable",
        action="store_true",
        help="After readiness, run probe and enable this endpoint as an APXM backend",
    )
    docker_start.add_argument(
        "--alias",
        action="append",
        default=[],
        dest=ArgName.ALIAS.value,
        help="Alternative routing alias to register when --enable is set",
    )
    docker_start.add_argument(
        "--container-env",
        action="append",
        default=[],
        dest=ArgName.CONTAINER_ENV.value,
        help="Pass KEY=VALUE, or a named host environment variable, into the container",
    )
    docker_start.set_defaults(**{ArgName.HANDLER.value: docker_start_cmd})

    for subcommand, help_text, handler in (
        (VllmCommand.DOCKER_STOP, "Stop and remove a Dekk-managed vLLM container", docker_stop_cmd),
        (VllmCommand.DOCKER_STATUS, "Show Dekk-managed vLLM container status", docker_status_cmd),
        (VllmCommand.DOCKER_LOGS, "Show Dekk-managed vLLM container logs", docker_logs_cmd),
    ):
        docker_parser = subparsers.add_parser(subcommand.value, help=help_text)
        docker_parser.add_argument("--port", type=int, default=None, help="vLLM port")
        docker_parser.add_argument(
            "--container-name",
            dest=ArgName.CONTAINER_NAME.value,
            help="Docker container name (resolved from --port's state file if omitted; required when no state)",
        )
        if subcommand == VllmCommand.DOCKER_LOGS:
            docker_parser.add_argument("--lines", type=int, default=DEFAULTS.log_lines, help="Number of lines to show")
            docker_parser.add_argument("--follow", action="store_true", help="Follow the log")
        docker_parser.set_defaults(**{ArgName.HANDLER.value: lambda ns, extra, h=handler: h(ns)})

    # Adopting an externally-started job is done by appending a [[deployment]]
    # entry to deploy/vllm/zoo.toml (or your own manifest) and running
    # `zoo apply`; idempotent reconciliation covers the "record an existing
    # job" workflow.

    service_start = subparsers.add_parser(
        VllmCommand.SERVICE_START.value,
        help="Submit a persistent Slurm-owned APXM-vLLM service job",
    )
    service_start.add_argument(ArgName.NAME.value, help="Service name stored under .apxm/vllm-services")
    service_start.add_argument(ArgName.MODEL.value, help="vLLM model id or local model path")
    service_start.add_argument(
        "--image",
        dest=ArgName.IMAGE.value,
        help="APXM-vLLM image tag/digest (default: APXM_VLLM_IMAGE or apxm-vllm-runtime:<apxm>-<vllm>)",
    )
    service_start.add_argument("--backend-name", default=DEFAULT_BACKEND_NAME, help="APXM backend name")
    service_start.add_argument("--port", type=int, default=None, help="vLLM port")
    service_start.add_argument("--served-model-name", dest=ArgName.SERVED_MODEL_NAME.value, help="Served model id")
    service_start.add_argument(
        "--hf-home",
        dest=ArgName.HF_HOME.value,
        help=f"Host Hugging Face cache root (or set {ENV_APXM_VLLM_HF_HOME}/{ENV_HF_HOME})",
    )
    service_start.add_argument(
        "--max-model-len",
        type=int,
        default=SERVICE_DEFAULTS.max_model_len,
        help="MAX_MODEL_LEN exported to the Slurm wrapper",
    )
    service_start.add_argument(
        "--max-num-seqs",
        dest=ArgName.MAX_NUM_SEQS.value,
        type=int,
        default=SERVICE_DEFAULTS.max_num_seqs,
        help="MAX_NUM_SEQS exported to the Slurm wrapper",
    )
    service_start.add_argument(
        VllmServeFlag.SCHEDULING_POLICY.value,
        dest=ArgName.SCHEDULING_POLICY.value,
        choices=[p.value for p in SchedulingPolicy],
        default=SERVICE_DEFAULTS.scheduling_policy,
        help="SCHEDULING_POLICY exported to the Slurm wrapper",
    )
    service_start.add_argument(
        "--enable-prefix-caching",
        dest=ArgName.ENABLE_PREFIX_CACHING.value,
        action=argparse.BooleanOptionalAction,
        default=SERVICE_DEFAULTS.enable_prefix_caching,
        help="ENABLE_PREFIX_CACHING exported to the Slurm wrapper",
    )
    service_start.add_argument(
        "--startup-timeout",
        type=float,
        default=SERVICE_DEFAULTS.startup_timeout_seconds,
        help="STARTUP_TIMEOUT_SECONDS exported to the Slurm wrapper",
    )
    service_start.add_argument(
        "--gpus",
        dest=ArgName.GPUS.value,
        help="GPU subset to pin (forwarded to the wrapper as HIP_VISIBLE_DEVICES/CUDA_VISIBLE_DEVICES)",
    )
    service_start.add_argument(
        "--tensor-parallel-size",
        dest=ArgName.TENSOR_PARALLEL_SIZE.value,
        type=int,
        help="TENSOR_PARALLEL_SIZE exported to the Slurm wrapper",
    )
    service_start.set_defaults(**{ArgName.HANDLER.value: lambda ns, extra: service_start_cmd(ns)})

    service_list = subparsers.add_parser(
        VllmCommand.SERVICE_LIST.value,
        help="List every recorded APXM-vLLM service and its current Slurm state",
    )
    service_list.set_defaults(**{ArgName.HANDLER.value: lambda ns, extra: service_list_cmd(ns)})

    for zoo_cmd, help_text in (
        (VllmCommand.ZOO_APPLY, "Reconcile the zoo manifest against currently-recorded services"),
        (VllmCommand.ZOO_STATUS, "Probe every service in the zoo manifest"),
        (VllmCommand.ZOO_SCALE, "Scale a zoo-managed service's replica count"),
        (VllmCommand.ZOO_CACHE_WARM, "Warm the HF cache for every model in the manifest"),
        (VllmCommand.ZOO_LOGS, "Tail container logs for a zoo-managed service"),
    ):
        zoo_parser = subparsers.add_parser(zoo_cmd.value, help=help_text)
        zoo_parser.add_argument(
            "manifest",
            nargs="?",
            default="deploy/vllm/zoo.toml",
            help=(
                "Path to the zoo manifest "
                "(default: deploy/vllm/zoo.toml — gitignored; copy "
                "deploy/vllm/zoo.example.toml to bootstrap)"
            ),
        )
        if zoo_cmd is VllmCommand.ZOO_APPLY:
            zoo_parser.add_argument(
                "--prune",
                dest=ArgName.PRUNE.value,
                action="store_true",
                help="Cancel services not present in the manifest (off by default to protect peer jobs)",
            )
        if zoo_cmd is VllmCommand.ZOO_SCALE:
            zoo_parser.add_argument(ArgName.NAME.value, help="Service name to scale")
            zoo_parser.add_argument(
                "--replicas",
                dest=ArgName.REPLICAS.value,
                type=int,
                required=True,
                help="Target replica count (0 = stop)",
            )
        zoo_parser.set_defaults(**{ArgName.HANDLER.value: lambda ns, extra, c=zoo_cmd: zoo_dispatch(ns, c)})

    service_status = subparsers.add_parser(
        VllmCommand.SERVICE_STATUS.value,
        help="Show a persistent APXM-vLLM service job",
    )
    service_status.add_argument(ArgName.NAME.value, help="Service name")
    service_status.add_argument("--probe", action="store_true", help="Run probe inside the service allocation")
    service_status.set_defaults(**{ArgName.HANDLER.value: lambda ns, extra: service_status_cmd(ns)})

    service_exec = subparsers.add_parser(
        VllmCommand.SERVICE_EXEC.value,
        help="Run a command inside a persistent APXM-vLLM service allocation",
    )
    service_exec.add_argument(ArgName.NAME.value, help="Service name")
    service_exec.add_argument("service_command", nargs=argparse.REMAINDER, help="Command to run after --")
    service_exec.set_defaults(**{ArgName.HANDLER.value: service_exec_cmd})

    service_stop = subparsers.add_parser(
        VllmCommand.SERVICE_STOP.value,
        help="Cancel a persistent APXM-vLLM service job",
    )
    service_stop.add_argument(ArgName.NAME.value, help="Service name")
    service_stop.add_argument("--remove-state", action="store_true", help="Delete the service state file after scancel")
    service_stop.set_defaults(**{ArgName.HANDLER.value: lambda ns, extra: service_stop_cmd(ns)})

    probe = subparsers.add_parser(
        VllmCommand.PROBE.value,
        help="Probe /models and APXM graph endpoints by registering and deleting a temporary graph",
    )
    probe.add_argument("--port", type=int, default=None, help="vLLM port")
    probe.add_argument("--endpoint", dest=ArgName.ENDPOINT.value, help="Override OpenAI-compatible endpoint")
    probe.add_argument(
        "--api-key",
        help=f"API key required by the vLLM server (or set {ENV_VLLM_API_KEY})",
    )
    probe.add_argument(
        "--graph-id",
        help="Temporary graph id to register and delete (default: generated)",
    )
    probe.set_defaults(**{ArgName.HANDLER.value: lambda ns, extra: probe_cmd(ns)})

    def add_registration_args(registration: argparse.ArgumentParser) -> None:
        registration.add_argument(ArgName.MODEL.value, help="Served model name exposed by /v1/models")
        registration.add_argument("--backend-name", default=DEFAULT_BACKEND_NAME, help="APXM backend name")
        registration.add_argument("--port", type=int, default=None, help="vLLM port")
        registration.add_argument("--endpoint", dest=ArgName.ENDPOINT.value, help="Override APXM backend endpoint")
        registration.add_argument(
            "--api-key",
            dest=ArgName.API_KEY.value,
            help=f"API key required for the enable preflight; persisted only through an env reference",
        )
        registration.add_argument(
            "--api-key-env",
            dest=ArgName.API_KEY_ENV.value,
            help=f"Environment variable name to persist as the backend API key reference (default: {ENV_VLLM_API_KEY} when set)",
        )
        registration.add_argument(
            "--alias",
            action="append",
            default=[],
            dest=ArgName.ALIAS.value,
            help="Alternative routing alias to register for the served model",
        )

    enable = subparsers.add_parser(VllmCommand.ENABLE.value, help="Enable running vLLM as an APXM backend")
    add_registration_args(enable)
    enable.set_defaults(**{ArgName.HANDLER.value: lambda ns, extra: enable_cmd(ns)})

    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args, extra_args = parser.parse_known_args(argv)
    if arg_value(args, ArgName.COMMAND) in COMMANDS_WITHOUT_EXTRA_ARGS and extra_args:
        parser.error(f"unrecognized arguments: {' '.join(extra_args)}")
    model = arg_value(args, ArgName.MODEL)
    if model is not None and str(model).startswith("-"):
        parser.error("model value must not start with '-'")
    return arg_value(args, ArgName.HANDLER)(args, extra_args)


if __name__ == "__main__":
    raise SystemExit(main())
