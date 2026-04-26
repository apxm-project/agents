#!/usr/bin/env python3
"""Operate the repo-local external/vllm fork without exposing its venv layout."""

from __future__ import annotations

import argparse
import json
import os
import signal
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

from apxm_vllm_contract import (
    ApiRoute,
    ArgName,
    BackendProtocol,
    BackendType,
    DekkToken,
    EnvVar,
    ForkModule,
    ProbeContract,
    ToolName,
    VllmCommand,
    VllmDefaults,
    VllmServeFlag,
    apxm_config_path,
    arg_value,
    build_layout,
    display_hf_home,
    effective_hf_home,
    env_name,
    env_reference,
    graph_register_path,
    local_endpoint,
)

LAYOUT = build_layout(__file__)
DEFAULTS = VllmDefaults()
PROBE = ProbeContract()
REPO_ROOT = LAYOUT.repo_root
VLLM_DIR = LAYOUT.vllm_dir
VLLM_PYTHON = LAYOUT.vllm_python
LOG_DIR = LAYOUT.log_dir
APXM_CONFIG = apxm_config_path()

DEFAULT_BACKEND_NAME = DEFAULTS.backend_name
DEFAULT_HOST = DEFAULTS.host
DEFAULT_PORT = DEFAULTS.port
DEFAULT_REQUEST_TIMEOUT_SECONDS = DEFAULTS.request_timeout_seconds
DEFAULT_STARTUP_TIMEOUT_SECONDS = DEFAULTS.startup_timeout_seconds
DEFAULT_STOP_TIMEOUT_SECONDS = DEFAULTS.stop_timeout_seconds
LOCALHOST = "127.0.0.1"
LOCALHOST_NAME = "localhost"
CONTENT_TYPE_HEADER = "content-type"
AUTHORIZATION_HEADER = "authorization"
JSON_CONTENT_TYPE = "application/json"
BEARER_AUTH_SCHEME = "Bearer"
MODELS_PATH = ApiRoute.MODELS.value
APXM_GRAPHS_PATH = ApiRoute.APXM_GRAPHS.value
APXM_GRAPH_REGISTER_PATH = graph_register_path()
PROBE_GRAPH_ID = PROBE.graph_id
TEMP_GRAPH_ID_PREFIX = PROBE.temp_graph_id_prefix
TEMP_EXECUTION_ID_PREFIX = PROBE.temp_execution_id_prefix
TEMP_NODE_NAME = PROBE.temp_node_name
ENV_HF_HOME = env_name(EnvVar.HF_HOME)
ENV_APXM_VLLM_HF_HOME = env_name(EnvVar.APXM_VLLM_HF_HOME)
ENV_VLLM_API_KEY = env_name(EnvVar.VLLM_API_KEY)
ENV_HIP_VISIBLE_DEVICES = env_name(EnvVar.HIP_VISIBLE_DEVICES)
ENV_CUDA_VISIBLE_DEVICES = env_name(EnvVar.CUDA_VISIBLE_DEVICES)
PID_STATE_VERSION = 1
PROC_ROOT = Path("/proc")
GIT = ToolName.GIT.value
LSOF = ToolName.LSOF.value
TAIL = ToolName.TAIL.value
UV = ToolName.UV.value
MANAGED_BY = "dekk apxm vllm"
COMMANDS_WITHOUT_EXTRA_ARGS = {
    VllmCommand.INSTALL.value,
    VllmCommand.HELP.value,
    VllmCommand.DOCTOR.value,
    VllmCommand.DOWNLOAD.value,
    VllmCommand.STOP.value,
    VllmCommand.STATUS.value,
    VllmCommand.LOGS.value,
    VllmCommand.PROBE.value,
    VllmCommand.ENABLE.value,
}

VERIFY_FORK_CODE = """
import importlib
import json
import sys
from pathlib import Path

expected = Path(sys.argv[1]).resolve()
contract = json.loads(sys.argv[2])
vllm = importlib.import_module(contract["vllm_module"])
router = importlib.import_module(contract["router_module"])
api_server = importlib.import_module(contract["api_server_module"])

got = Path(vllm.__file__).resolve().parent.parent
if got != expected:
    print(
        f"ERROR: vllm imports from {got}, expected fork at {expected}",
        file=sys.stderr,
    )
    sys.exit(1)

router_path = Path(router.__file__).resolve()
if expected not in router_path.parents:
    print(
        f"ERROR: APXM router imports from {router_path}, expected under {expected}",
        file=sys.stderr,
    )
    sys.exit(1)

api_server_text = Path(api_server.__file__).read_text(encoding="utf-8")
if contract["router_module"] not in api_server_text:
    print(
        "ERROR: OpenAI API server does not mount the APXM router from this checkout.",
        file=sys.stderr,
    )
    sys.exit(1)

print("OK: editable install resolves to external/vllm fork")
print(f"OK: APXM router imports from {router_path}")
"""

DOCTOR_CODE = """
import importlib
import json
import sys
from pathlib import Path

payload = {}
for name in json.loads(sys.argv[1])["modules"]:
    try:
        module = importlib.import_module(name)
        payload[name] = {
            "version": getattr(module, "__version__", "unknown"),
            "file": str(Path(getattr(module, "__file__", "")).resolve()),
        }
    except Exception as exc:
        payload[name] = {"error": repr(exc)}

try:
    import torch
    payload["torch"].update(
        {
            "hip": getattr(torch.version, "hip", None),
            "cuda": torch.version.cuda,
            "cuda_available": torch.cuda.is_available(),
            "device_count": torch.cuda.device_count(),
        }
    )
except Exception:
    pass

try:
    router = importlib.import_module(json.loads(sys.argv[1])["router_module"])
    payload["apxm_router"] = str(Path(router.__file__).resolve())
except Exception as exc:
    payload["apxm_router_error"] = repr(exc)

print(json.dumps(payload, indent=2, sort_keys=True))
"""

DOWNLOAD_CODE = """
import sys
from huggingface_hub import snapshot_download

model = sys.argv[1]
max_workers = int(sys.argv[2])
path = snapshot_download(
    model,
    max_workers=max_workers,
)
print(path)
"""


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


def _ensure_installed() -> bool:
    if VLLM_PYTHON.exists():
        return True
    _print("external/vllm is not installed yet.")
    _print("From the repo root, run: dekk apxm vllm install")
    return False


def _verify_visible_fork(*, verbose: bool) -> bool:
    if not _ensure_installed():
        return False
    result = _capture(
        [str(VLLM_PYTHON), "-c", VERIFY_FORK_CODE, str(VLLM_DIR), _fork_contract_payload()],
        cwd=VLLM_DIR,
    )
    if result.returncode == 0:
        if verbose and result.stdout.strip():
            _print(result.stdout.strip())
        return True

    if result.stdout.strip():
        _print(result.stdout.strip())
    if result.stderr.strip():
        _print(result.stderr.strip())
    _print("From the repo root, run: dekk apxm vllm install")
    return False


def _hf_home(args: argparse.Namespace) -> str | None:
    return effective_hf_home(explicit=arg_value(args, ArgName.HF_HOME))


def _serve_env(args: argparse.Namespace) -> dict[str, str]:
    env = dict(os.environ)
    hf_home = _hf_home(args)
    if hf_home:
        env[ENV_HF_HOME] = hf_home
    if args.gpus:
        env[ENV_HIP_VISIBLE_DEVICES] = args.gpus
        env[ENV_CUDA_VISIBLE_DEVICES] = args.gpus
    if arg_value(args, ArgName.API_KEY):
        env[ENV_VLLM_API_KEY] = str(arg_value(args, ArgName.API_KEY))
    return env


def _api_key(args: argparse.Namespace) -> str | None:
    api_key_env = arg_value(args, ArgName.API_KEY_ENV)
    return (
        arg_value(args, ArgName.API_KEY)
        or (os.environ.get(api_key_env) if api_key_env else None)
        or os.environ.get(ENV_VLLM_API_KEY)
    )


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
    return _normalize_endpoint(arg_value(args, ArgName.ENDPOINT) or _endpoint(args.port))


def _normalize_endpoint(endpoint: str) -> str:
    normalized = endpoint.rstrip("/")
    normalized = normalized.replace(f"//{LOCALHOST_NAME}:", f"//{LOCALHOST}:")
    if not normalized.endswith(f"/{ApiRoute.OPENAI_PREFIX.value}"):
        normalized = f"{normalized}/{ApiRoute.OPENAI_PREFIX.value}"
    return normalized


def _temporary_id(prefix: str) -> str:
    return f"{prefix}-{uuid.uuid4()}"


def _pid_file(port: int) -> Path:
    return LOG_DIR / f"server-{port}.pid"


def _pid_state_file(port: int) -> Path:
    return LOG_DIR / f"server-{port}.json"


def _log_file(port: int) -> Path:
    return LOG_DIR / f"server-{port}.log"


def _read_pid(path: Path) -> int | None:
    try:
        return int(path.read_text(encoding="utf-8").strip())
    except (OSError, ValueError):
        return None


def _pid_is_running(pid: int) -> bool:
    try:
        os.kill(pid, 0)
        return True
    except OSError:
        return False


def _process_cmdline(pid: int) -> list[str]:
    if not PROC_ROOT.exists():
        return []
    try:
        raw = (PROC_ROOT / str(pid) / "cmdline").read_bytes()
    except OSError:
        return []
    return [part.decode("utf-8", errors="replace") for part in raw.split(b"\0") if part]


def _process_cwd(pid: int) -> Path | None:
    if not PROC_ROOT.exists():
        return None
    try:
        return (PROC_ROOT / str(pid) / "cwd").resolve()
    except OSError:
        return None


def _is_repo_vllm_process(pid: int) -> bool:
    if not _pid_is_running(pid):
        return False
    cwd = _process_cwd(pid)
    cmdline = _process_cmdline(pid)
    if cwd != VLLM_DIR or not cmdline:
        return False
    return (
        str(VLLM_PYTHON) in cmdline
        and ForkModule.VLLM_CLI.value in cmdline
        and VllmCommand.SERVE.value in cmdline
    )


def _read_pid_state(port: int) -> dict[str, Any] | None:
    try:
        state = json.loads(_pid_state_file(port).read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return None
    return state if isinstance(state, dict) else None


def _state_matches_managed_process(port: int, pid: int) -> bool:
    state = _read_pid_state(port)
    if not state:
        return False
    return (
        state.get("version") == PID_STATE_VERSION
        and state.get("managed_by") == MANAGED_BY
        and state.get("pid") == pid
        and state.get("port") == port
        and state.get("cwd") == str(VLLM_DIR)
        and state.get("python") == str(VLLM_PYTHON)
    )


def _is_managed_repo_vllm_process(port: int, pid: int) -> bool:
    if not _pid_is_running(pid):
        return False
    if PROC_ROOT.exists():
        return _is_repo_vllm_process(pid)
    if _state_matches_managed_process(port, pid):
        _print(
            "Process metadata is not available on this host; trusting the "
            "Dekk-managed pid state file."
        )
        return True
    return False


def _write_pid_state(args: argparse.Namespace, process: subprocess.Popen[Any]) -> None:
    state = {
        "version": PID_STATE_VERSION,
        "managed_by": MANAGED_BY,
        "pid": process.pid,
        "port": args.port,
        "model": args.model,
        "served_model_name": args.served_model_name or args.model,
        "backend_name": args.backend_name,
        "api_key_configured": bool(_api_key(args)),
        "cwd": str(VLLM_DIR),
        "python": str(VLLM_PYTHON),
        "endpoint": _endpoint_for_args(args),
        "started_at": time.time(),
    }
    _pid_file(args.port).write_text(f"{process.pid}\n", encoding="utf-8")
    _pid_state_file(args.port).write_text(
        json.dumps(state, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )


def _remove_pid_state(port: int) -> None:
    for path in (_pid_file(port), _pid_state_file(port)):
        try:
            path.unlink()
        except FileNotFoundError:
            pass
        except OSError as exc:
            _print(f"Could not remove {path}: {exc}")


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
    probe_host = host if host not in ("", "0.0.0.0") else LOCALHOST
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
    method: str = "GET",
    body: dict[str, Any] | None = None,
    api_key: str | None = None,
) -> Any:
    data = None
    headers = {CONTENT_TYPE_HEADER: JSON_CONTENT_TYPE}
    if api_key:
        headers[AUTHORIZATION_HEADER] = f"{BEARER_AUTH_SCHEME} {api_key}"
    if body is not None:
        data = json.dumps(body).encode("utf-8")
    request = urllib.request.Request(url, data=data, headers=headers, method=method)
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
    except (urllib.error.URLError, TimeoutError, json.JSONDecodeError) as exc:
        _print(f"Enable preflight failed for {base}: {exc}")
        return False
    return True


def _build_serve_cmd(args: argparse.Namespace, extra_args: list[str]) -> tuple[list[str], dict[str, str]]:
    served_model_name = args.served_model_name or args.model
    cmd = [
        str(VLLM_PYTHON),
        "-m",
        ForkModule.VLLM_CLI.value,
        VllmCommand.SERVE.value,
        args.model,
        "--served-model-name",
        served_model_name,
        "--host",
        args.host,
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

    return cmd, _serve_env(args)


def _print_registration_hint(args: argparse.Namespace) -> None:
    served_model_name = args.served_model_name or args.model
    endpoint = _endpoint_for_args(args)
    _print("")
    _print("Register this vLLM backend with APXM after the server is reachable:")
    _print(
        f"  dekk apxm vllm enable {served_model_name} "
        f"--backend-name {args.backend_name} --port {args.port} --endpoint {endpoint}"
    )
    if _api_key(args):
        _print(f"  Keep {ENV_VLLM_API_KEY} set, or add --api-key-env <ENV_VAR> when enabling.")
    _print("")


def _warn_if_public_bind_without_key(args: argparse.Namespace) -> None:
    if args.host in {"0.0.0.0", "::"} and not _api_key(args):
        _print(
            "Warning: binding vLLM on a wildcard host without an API key. "
            f"For shared or remote hosts, set {ENV_VLLM_API_KEY} or pass --api-key."
        )


def install_cmd(_args: argparse.Namespace) -> int:
    if not (VLLM_DIR / "pyproject.toml").exists():
        rc = _run(
            [GIT, "-C", str(REPO_ROOT), "submodule", "update", "--init", str(VLLM_DIR.relative_to(REPO_ROOT))]
        )
        if rc != 0:
            return rc
    if not (VLLM_DIR / "pyproject.toml").exists():
        _print("external/vllm is not available after submodule init")
        return 1
    if not shutil.which(UV):
        _print("uv is required for the repo-local vLLM install.")
        _print("Install uv through your system/Dekk environment, then rerun: dekk apxm vllm install")
        return 1
    if not VLLM_PYTHON.exists():
        rc = _run([UV, "venv", "--python", sys.executable, str(LAYOUT.venv_dir)], cwd=VLLM_DIR)
        if rc != 0:
            return rc
    install_steps = [
        [
            UV,
            "pip",
            "install",
            "--python",
            str(VLLM_PYTHON),
            "--torch-backend=auto",
            "-r",
            "requirements/build.txt",
        ],
        [
            UV,
            "pip",
            "install",
            "--python",
            str(VLLM_PYTHON),
            "-e",
            ".",
            "--torch-backend=auto",
            "--no-build-isolation",
        ],
    ]
    for step in install_steps:
        rc = _run(step, cwd=VLLM_DIR)
        if rc != 0:
            return rc
    for label, cmd in (
        ("HEAD", [GIT, "-C", str(VLLM_DIR), "rev-parse", "--short", "HEAD"]),
        ("origin", [GIT, "-C", str(VLLM_DIR), "remote", "get-url", "origin"]),
    ):
        result = _capture(cmd)
        if result.returncode == 0 and result.stdout.strip():
            _print(f"external/vllm {label}={result.stdout.strip()}")
    if not _verify_visible_fork(verbose=True):
        return 1
    _print("Installed the repo-local fork from external/vllm.")
    _print("Next: dekk apxm vllm start <MODEL_REF> --served-model-name <SERVED_MODEL_ID> --wait")
    return 0


def help_cmd(args: argparse.Namespace) -> int:
    cmd = [sys.executable, str(Path(__file__).resolve())]
    if args.topic:
        cmd.append(args.topic)
    cmd.append("--help")
    return _run(cmd, cwd=REPO_ROOT)


def doctor_cmd(args: argparse.Namespace) -> int:
    if not _verify_visible_fork(verbose=True):
        return 1
    result = _capture([str(VLLM_PYTHON), "-c", DOCTOR_CODE, _doctor_payload()], cwd=VLLM_DIR)
    if result.stdout.strip():
        print(result.stdout.strip())
    if result.stderr.strip():
        _print(result.stderr.strip())
    print(f"HF_HOME={display_hf_home(_hf_home(args))}")
    print(f"APXM_VLLM_HF_HOME={os.environ.get(ENV_APXM_VLLM_HF_HOME, '')}")
    print(f"server_pid_file={_pid_file(args.port)}")
    print(f"server_log_file={_log_file(args.port)}")
    pids = _port_pids(args.port)
    print(f"port_{args.port}_pids={','.join(map(str, pids)) if pids else ''}")
    return result.returncode


def download_cmd(args: argparse.Namespace) -> int:
    if not _verify_visible_fork(verbose=True):
        return 1
    hf_home = _hf_home(args)
    env = dict(os.environ)
    if hf_home:
        Path(hf_home).mkdir(parents=True, exist_ok=True)
        env[ENV_HF_HOME] = hf_home
    _print(f"Downloading {args.model} with HF_HOME={display_hf_home(hf_home)}")
    return _run(
        [str(VLLM_PYTHON), "-c", DOWNLOAD_CODE, args.model, str(args.max_workers)],
        cwd=VLLM_DIR,
        env=env,
    )


def serve_cmd(args: argparse.Namespace, extra_args: list[str]) -> int:
    if not _verify_visible_fork(verbose=True):
        return 1

    cmd, env = _build_serve_cmd(args, extra_args)
    _warn_if_public_bind_without_key(args)
    _print("Launching the visible repo-local fork from external/vllm")
    _print(f"working_dir={VLLM_DIR}")
    _print(f"python={VLLM_PYTHON}")
    _print(f"hf_home={display_hf_home(env.get(ENV_HF_HOME))} (used for Hugging Face-backed model refs)")
    _print(f"backend_name={args.backend_name}")
    _print(f"model={args.model}")
    _print(f"registration_endpoint={_endpoint_for_args(args)}")
    _print_registration_hint(args)
    return _run(cmd, cwd=VLLM_DIR, env=env)


def start_cmd(args: argparse.Namespace, extra_args: list[str]) -> int:
    if not _verify_visible_fork(verbose=True):
        return 1
    pids = _port_pids(args.port)
    if pids or not _port_is_available(args.host, args.port):
        _print(f"Port {args.port} is already in use by {_port_owner_hint(args.port)}")
        return 1

    LOG_DIR.mkdir(parents=True, exist_ok=True)
    cmd, env = _build_serve_cmd(args, extra_args)
    _warn_if_public_bind_without_key(args)
    log_path = _log_file(args.port)
    with log_path.open("ab") as log:
        process = subprocess.Popen(
            cmd,
            cwd=VLLM_DIR,
            env=env,
            stdout=log,
            stderr=subprocess.STDOUT,
            start_new_session=True,
        )
    _write_pid_state(args, process)
    _print(f"Started vLLM PID {process.pid}")
    _print(f"log={log_path}")
    _print(f"pid_file={_pid_file(args.port)}")
    _print(f"registration_endpoint={_endpoint_for_args(args)}")
    _print_registration_hint(args)
    if args.wait:
        deadline = time.time() + args.startup_timeout
        while time.time() < deadline:
            if not _pid_is_running(process.pid):
                _print(f"vLLM process exited before the server became ready; see {log_path}")
                return 1
            if _server_ready(_endpoint_for_args(args), api_key=_api_key(args)):
                _print(f"Server responded on local_probe_endpoint={_endpoint_for_args(args)}")
                return 0
            time.sleep(2.0)
        _print(
            f"Timed out waiting for local_probe_endpoint={_endpoint(args.port)}; "
            f"PID {process.pid} may still be starting. See {log_path}, or run "
        f"`dekk apxm vllm status --port {args.port}` / "
        f"`dekk apxm vllm stop --port {args.port}`."
        )
        return 1
    return 0


def stop_cmd(args: argparse.Namespace) -> int:
    pids: list[int] = []
    pid = _read_pid(_pid_file(args.port))
    if pid is not None:
        if _is_managed_repo_vllm_process(args.port, pid):
            pids.append(pid)
        else:
            _print(f"Refusing stale or unmanaged pid-file PID {pid}")
    skipped_port_pids: list[int] = []
    for port_pid in _port_pids(args.port):
        if _is_managed_repo_vllm_process(args.port, port_pid):
            pids.append(port_pid)
        else:
            skipped_port_pids.append(port_pid)
    pids = sorted(set(pids))
    if not pids:
        if skipped_port_pids:
            _print(
                f"Port {args.port} is owned by unmanaged PID(s): "
                f"{', '.join(map(str, skipped_port_pids))}"
            )
            _print(
                "This command only stops repo-local external/vllm processes. "
                "Use a separate system administration command for unrelated owners."
            )
            return 1
        _print(f"No vLLM process found for port {args.port}")
        return 0

    for pid in pids:
        try:
            os.kill(pid, signal.SIGTERM)
            _print(f"Sent SIGTERM to PID {pid}")
        except OSError as exc:
            _print(f"PID {pid}: {exc}")
    deadline = time.time() + args.timeout
    while time.time() < deadline:
        if not any(_pid_is_running(pid) for pid in pids):
            break
        time.sleep(0.5)
    for pid in pids:
        if _pid_is_running(pid):
            try:
                os.kill(pid, signal.SIGKILL)
                _print(f"Sent SIGKILL to PID {pid}")
            except OSError:
                pass
    if not any(_pid_is_running(pid) for pid in pids):
        _remove_pid_state(args.port)
    return 0


def status_cmd(args: argparse.Namespace) -> int:
    registration_endpoint = None
    backend_name = None
    served_model_name = None
    api_key_configured = None
    state = _read_pid_state(args.port)
    if state:
        registration_endpoint = state.get("endpoint")
        backend_name = state.get("backend_name")
        served_model_name = state.get("served_model_name")
        api_key_configured = state.get("api_key_configured")
    pid = _read_pid(_pid_file(args.port))
    if pid is not None:
        print(f"pid_file={_pid_file(args.port)}")
        print(f"pid={pid}")
        print(f"pid_running={str(_pid_is_running(pid)).lower()}")
    pids = _port_pids(args.port)
    print(f"port={args.port}")
    print(f"port_available={str(_port_is_available(LOCALHOST, args.port)).lower()}")
    print(f"port_pids={','.join(map(str, pids)) if pids else ''}")
    if not pids and not shutil.which(LSOF):
        print(f"port_pid_attribution={LSOF}_not_available")
    print(f"local_probe_endpoint={_endpoint(args.port)}")
    if registration_endpoint:
        print(f"registration_endpoint={registration_endpoint}")
    if backend_name:
        print(f"backend_name={backend_name}")
    if served_model_name:
        print(f"served_model_name={served_model_name}")
    if api_key_configured is not None:
        print(f"api_key_configured={str(bool(api_key_configured)).lower()}")
    print(f"log={_log_file(args.port)}")
    return 0


def logs_cmd(args: argparse.Namespace) -> int:
    path = _log_file(args.port)
    if not path.exists():
        _print(f"No log file found at {path}")
        return 1
    if args.follow:
        if shutil.which(TAIL):
            return _run([TAIL, "-f", str(path)], cwd=REPO_ROOT)
        _print(f"{TAIL} is not available on this host; use --lines without --follow.")
        return 1
    try:
        lines = path.read_text(encoding="utf-8", errors="replace").splitlines()
    except OSError as exc:
        _print(f"Could not read {path}: {exc}")
        return 1
    for line in lines[-args.lines :]:
        print(line)
    return 0


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

        registration = _http_json(
            f"{base}/{APXM_GRAPH_REGISTER_PATH}",
            method="POST",
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
        release = _http_json(f"{base}/{APXM_GRAPHS_PATH}/{graph_id}", method="DELETE", api_key=api_key)
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
                _http_json(f"{base}/{APXM_GRAPHS_PATH}/{graph_id}", method="DELETE", api_key=api_key)
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
    if not args.skip_test and not _verify_enable_target(endpoint, model, api_key=api_key):
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
        else:
            _print(f"Backend {args.backend_name} already exists; skipping backend add.")
    elif _run(add, cwd=REPO_ROOT) != 0:
        return 1

    if _backend_model_exists(args.backend_name, model):
        _print(f"Model {model} already exists on backend {args.backend_name}; skipping model add.")
    elif _run(add_model, cwd=REPO_ROOT) != 0:
        return 1
    if args.skip_test:
        _print("Skipping backend test by request.")
        return 0
    return _run(test, cwd=REPO_ROOT)


def _add_model_args(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("model", help="vLLM model id or local model path")
    parser.add_argument("--backend-name", default=DEFAULT_BACKEND_NAME, help="APXM backend name")
    parser.add_argument(
        "--host",
        default=DEFAULT_HOST,
        help="vLLM bind host; does not change the APXM registration endpoint",
    )
    parser.add_argument("--port", type=int, default=DEFAULT_PORT, help="Bind port for vLLM")
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
        help="Enable a vLLM reasoning parser, for example gemma4 for Gemma 4 thinking models",
    )
    parser.add_argument(
        "--default-chat-template-kwargs",
        dest=ArgName.DEFAULT_CHAT_TEMPLATE_KWARGS.value,
        help="JSON object forwarded to vLLM as default chat-template kwargs",
    )
    parser.add_argument(
        "--enable-prompt-tokens-details",
        dest=ArgName.ENABLE_PROMPT_TOKENS_DETAILS.value,
        action="store_true",
        help="Ask vLLM to include prompt_tokens_details such as cached_tokens in usage",
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
        "--scheduling-policy",
        dest=ArgName.SCHEDULING_POLICY.value,
        help="vLLM scheduler policy, for example priority for APXM priority hints",
    )
    parser.add_argument(
        "--trust-remote-code",
        action="store_true",
        help="Pass --trust-remote-code to vLLM",
    )


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="dekk apxm vllm",
        description="Operate the repo-local external/vllm fork.",
    )
    subparsers = parser.add_subparsers(dest=ArgName.COMMAND.value, required=True)

    install = subparsers.add_parser(VllmCommand.INSTALL.value, help="Install the repo-local vLLM fork")
    install.set_defaults(**{ArgName.HANDLER.value: lambda ns, extra: install_cmd(ns)})

    help_parser = subparsers.add_parser(VllmCommand.HELP.value, help="Show controller or subcommand help")
    help_parser.add_argument("topic", nargs="?", help="Subcommand to describe")
    help_parser.set_defaults(**{ArgName.HANDLER.value: lambda ns, extra: help_cmd(ns)})

    doctor = subparsers.add_parser(VllmCommand.DOCTOR.value, help="Verify fork, Python packages, GPU, and port")
    doctor.add_argument(
        "--hf-home",
        help=f"HF_HOME value to report/use for Hugging Face-backed refs (or set {ENV_APXM_VLLM_HF_HOME}/{ENV_HF_HOME})",
    )
    doctor.add_argument("--port", type=int, default=DEFAULT_PORT, help="vLLM port")
    doctor.set_defaults(**{ArgName.HANDLER.value: lambda ns, extra: doctor_cmd(ns)})

    download = subparsers.add_parser(VllmCommand.DOWNLOAD.value, help="Download Hugging Face model weights into HF_HOME")
    download.add_argument(ArgName.MODEL.value, help="Hugging Face model id to download")
    download.add_argument(
        "--hf-home",
        dest=ArgName.HF_HOME.value,
        help=f"Hugging Face cache root (or set {ENV_APXM_VLLM_HF_HOME}/{ENV_HF_HOME})",
    )
    download.add_argument("--max-workers", type=int, default=DEFAULTS.download_workers, help="Parallel download workers")
    download.set_defaults(**{ArgName.HANDLER.value: lambda ns, extra: download_cmd(ns)})

    serve = subparsers.add_parser(VllmCommand.SERVE.value, help="Serve a model in the foreground")
    _add_model_args(serve)
    serve.set_defaults(**{ArgName.HANDLER.value: serve_cmd})

    start = subparsers.add_parser(VllmCommand.START.value, help="Start a model server in the background")
    _add_model_args(start)
    start.add_argument(
        "--wait",
        dest=ArgName.WAIT.value,
        action="store_true",
        default=True,
        help="Wait until /v1/models responds (default)",
    )
    start.add_argument(
        "--no-wait",
        dest=ArgName.WAIT.value,
        action="store_false",
        help="Return after launching the background process",
    )
    start.add_argument(
        "--startup-timeout",
        type=float,
        default=DEFAULT_STARTUP_TIMEOUT_SECONDS,
        help="Seconds to wait when --wait is set",
    )
    start.set_defaults(**{ArgName.HANDLER.value: start_cmd})

    stop = subparsers.add_parser(VllmCommand.STOP.value, help="Stop a repo-local vLLM server for a port")
    stop.add_argument("--port", type=int, default=DEFAULT_PORT, help="vLLM port")
    stop.add_argument("--timeout", type=float, default=DEFAULT_STOP_TIMEOUT_SECONDS, help="Seconds before SIGKILL")
    stop.set_defaults(**{ArgName.HANDLER.value: lambda ns, extra: stop_cmd(ns)})

    status = subparsers.add_parser(VllmCommand.STATUS.value, help="Show process and port status")
    status.add_argument("--port", type=int, default=DEFAULT_PORT, help="vLLM port")
    status.set_defaults(**{ArgName.HANDLER.value: lambda ns, extra: status_cmd(ns)})

    logs = subparsers.add_parser(VllmCommand.LOGS.value, help="Show background server logs")
    logs.add_argument("--port", type=int, default=DEFAULT_PORT, help="vLLM port")
    logs.add_argument("--lines", type=int, default=DEFAULTS.log_lines, help="Number of lines to show")
    logs.add_argument("--follow", action="store_true", help="Follow the log")
    logs.set_defaults(**{ArgName.HANDLER.value: lambda ns, extra: logs_cmd(ns)})

    probe = subparsers.add_parser(
        VllmCommand.PROBE.value,
        help="Probe /models and APXM graph endpoints by registering and deleting a temporary graph",
    )
    probe.add_argument("--port", type=int, default=DEFAULT_PORT, help="vLLM port")
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
        registration.add_argument("--port", type=int, default=DEFAULT_PORT, help="vLLM port")
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
        registration.add_argument("--skip-test", action="store_true", help="Write backend/model config without probing the server")

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
