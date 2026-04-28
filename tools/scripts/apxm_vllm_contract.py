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
    APXM_VLLM_HF_HOME = "APXM_VLLM_HF_HOME"
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

    ALIAS = "alias"
    API_KEY = "api_key"
    API_KEY_ENV = "api_key_env"
    COMMAND = "command"
    DEFAULT_CHAT_TEMPLATE_KWARGS = "default_chat_template_kwargs"
    ENDPOINT = "endpoint"
    ENABLE_FORCE_INCLUDE_USAGE = "enable_force_include_usage"
    ENABLE_PREFIX_CACHING = "enable_prefix_caching"
    ENABLE_PROMPT_TOKENS_DETAILS = "enable_prompt_tokens_details"
    HANDLER = "handler"
    HF_HOME = "hf_home"
    MODEL = "model"
    REASONING_PARSER = "reasoning_parser"
    SCHEDULING_POLICY = "scheduling_policy"
    SERVED_MODEL_NAME = "served_model_name"
    WAIT = "wait"


class VllmServeFlag(str, Enum):
    """vLLM server CLI flags forwarded by the controller."""

    DEFAULT_CHAT_TEMPLATE_KWARGS = "--default-chat-template-kwargs"
    ENABLE_FORCE_INCLUDE_USAGE = "--enable-force-include-usage"
    ENABLE_PREFIX_CACHING = "--enable-prefix-caching"
    ENABLE_PROMPT_TOKENS_DETAILS = "--enable-prompt-tokens-details"
    REASONING_PARSER = "--reasoning-parser"
    SCHEDULING_POLICY = "--scheduling-policy"


class SchedulingPolicy(str, Enum):
    """vLLM scheduler policy values understood by the APXM fork.

    The fork's per-request critical-path boost only takes effect under
    PRIORITY mode; FCFS silently ignores per-request priority hints. The
    APXM-fork default is PRIORITY (see external/vllm/vllm/config/scheduler.py).
    """

    FCFS = "fcfs"
    PRIORITY = "priority"


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
    # APXM ships with priority on by default so compiler-stamped critical-path
    # hints actually reorder the waiting queue. Operators can override by
    # passing --scheduling-policy fcfs explicitly.
    scheduling_policy: str = SchedulingPolicy.PRIORITY.value
    # APXM ships with prompt_tokens_details on by default so the OpenAI parser
    # at apxm-backends/src/llm/backends/openai/backend.rs:779-789 can populate
    # cached_input_tokens per request from usage.prompt_tokens_details.cached_tokens.
    # The fork's emission is gated at vllm/entrypoints/openai/chat_completion/serving.py
    # by --enable-prompt-tokens-details (default False upstream). Operators can
    # opt out with --no-enable-prompt-tokens-details.
    enable_prompt_tokens_details: bool = True


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


def apxm_config_path(start: Path | None = None) -> Path:
    explicit = os.environ.get(env_name(EnvVar.APXM_CONFIG), "").strip()
    if explicit:
        return Path(explicit)

    cwd = (start or Path.cwd()).resolve()
    for candidate_root in (cwd, *cwd.parents):
        candidate = candidate_root / ".apxm" / "config.toml"
        if candidate.is_file():
            return candidate

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
