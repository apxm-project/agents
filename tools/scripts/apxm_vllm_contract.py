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

    APXM_VLLM_BACKEND = "APXM_VLLM_BACKEND"
    APXM_VLLM_HF_HOME = "APXM_VLLM_HF_HOME"
    APXM_VLLM_MODEL = "APXM_VLLM_MODEL"
    CUDA_VISIBLE_DEVICES = "CUDA_VISIBLE_DEVICES"
    HF_HOME = "HF_HOME"
    HIP_VISIBLE_DEVICES = "HIP_VISIBLE_DEVICES"
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
    MODELS = "models"
    APXM_GRAPHS = "apxm/graphs"
    APXM_REGISTER = "register"


class ForkModule(str, Enum):
    """Python module names that prove the repo-local fork is in use."""

    VLLM = "vllm"
    APXM_ROUTER = "vllm.entrypoints.openai.apxm.api_router"
    OPENAI_API_SERVER = "vllm.entrypoints.openai.api_server"
    VLLM_CLI = "vllm.entrypoints.cli.main"


class ToolName(str, Enum):
    """External command names used by the controller."""

    GIT = "git"
    LSOF = "lsof"
    TAIL = "tail"
    UV = "uv"


class VllmCommand(str, Enum):
    """Subcommands exposed under `dekk apxm vllm`."""

    INSTALL = "install"
    HELP = "help"
    DOCTOR = "doctor"
    DOWNLOAD = "download"
    SERVE = "serve"
    START = "start"
    STOP = "stop"
    STATUS = "status"
    LOGS = "logs"
    PROBE = "probe"
    ENABLE = "enable"


class ArgName(str, Enum):
    """argparse destination names used by the controller."""

    API_KEY = "api_key"
    API_KEY_ENV = "api_key_env"
    COMMAND = "command"
    ENDPOINT = "endpoint"
    HANDLER = "handler"
    HF_HOME = "hf_home"
    MODEL = "model"
    SERVED_MODEL_NAME = "served_model_name"
    WAIT = "wait"


@dataclass(frozen=True)
class VllmDefaults:
    """Controller defaults that are model-neutral and safe for local hosts."""

    backend_name: str = "vllm-fork"
    host: str = "127.0.0.1"
    port: int = 8916
    request_timeout_seconds: int = 15
    startup_timeout_seconds: int = 900
    stop_timeout_seconds: float = 20.0
    download_workers: int = 8
    log_lines: int = 80


@dataclass(frozen=True)
class ProbeContract:
    """Temporary graph ids and node names used by `dekk apxm vllm probe`."""

    graph_id: str = "__apxm_probe__"
    temp_graph_id_prefix: str = "dekk-probe"
    temp_execution_id_prefix: str = "dekk-probe-exec"
    temp_node_name: str = "dekk-probe"


@dataclass(frozen=True)
class RepoLayout:
    """Resolved repo-local paths for the visible external/vllm fork."""

    repo_root: Path
    vllm_dir: Path
    venv_dir: Path
    vllm_python: Path
    log_dir: Path


def enum_value(value: str | Enum) -> str:
    return value.value if isinstance(value, Enum) else value


def arg_value(namespace: object, arg_name: ArgName, default: object = None) -> object:
    return getattr(namespace, arg_name.value, default)


def env_name(env_var: EnvVar) -> str:
    return env_var.value


def env_reference(env_var: EnvVar | str) -> str:
    return f"env:{enum_value(env_var)}"


def build_layout(script_file: str) -> RepoLayout:
    script_dir = Path(script_file).resolve().parent
    repo_root = script_dir.parent.parent
    vllm_dir = repo_root / "external" / "vllm"
    venv_dir = vllm_dir / ".venv"
    scripts_dir = "Scripts" if os.name == "nt" else "bin"
    python_name = "python.exe" if os.name == "nt" else "python"
    return RepoLayout(
        repo_root=repo_root,
        vllm_dir=vllm_dir,
        venv_dir=venv_dir,
        vllm_python=venv_dir / scripts_dir / python_name,
        log_dir=repo_root / ".apxm" / "vllm-logs",
    )


def apxm_config_path() -> Path:
    return Path.home() / ".apxm" / "config.toml"


def effective_hf_home(
    *,
    explicit: str | None = None,
    environ: dict[str, str] | os._Environ[str] = os.environ,
) -> str | None:
    """Resolve HF cache policy without inventing a machine-specific path."""

    return (
        explicit
        or environ.get(env_name(EnvVar.APXM_VLLM_HF_HOME))
        or environ.get(env_name(EnvVar.HF_HOME))
    )


def display_hf_home(value: str | None) -> str:
    return value if value else "<huggingface default>"


def local_endpoint(*, host: str, port: int) -> str:
    return f"http://{host}:{port}/{ApiRoute.OPENAI_PREFIX.value}"


def graph_register_path() -> str:
    return f"{ApiRoute.APXM_GRAPHS.value}/{ApiRoute.APXM_REGISTER.value}"
