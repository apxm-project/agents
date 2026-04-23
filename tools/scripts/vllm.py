#!/usr/bin/env python3
"""Operate the repo-local external/vllm fork without shell PATH setup."""

from __future__ import annotations

import argparse
import os
import subprocess
import sys
from pathlib import Path


SCRIPT_DIR = Path(__file__).resolve().parent
REPO_ROOT = SCRIPT_DIR.parent.parent
VLLM_DIR = REPO_ROOT / "external" / "vllm"
VLLM_PYTHON = VLLM_DIR / ".venv" / "bin" / "python"
INSTALL_SCRIPT = REPO_ROOT / "tools" / "scripts" / "install_external_vllm.sh"
DEFAULT_BACKEND_NAME = "vllm-fork"
DEFAULT_HOST = "0.0.0.0"
DEFAULT_PORT = 8916


def _run(cmd: list[str], *, cwd: Path | None = None, env: dict[str, str] | None = None) -> int:
    result = subprocess.run(cmd, cwd=cwd, env=env)
    return result.returncode


def _print(msg: str) -> None:
    print(msg, file=sys.stderr)


def install_cmd(_args: argparse.Namespace) -> int:
    return _run(["bash", str(INSTALL_SCRIPT)], cwd=REPO_ROOT)


def serve_cmd(args: argparse.Namespace, extra_args: list[str]) -> int:
    if not VLLM_PYTHON.exists():
        _print("external/vllm is not installed yet.")
        _print("Run: dekk apxm vllm install")
        return 1

    env = dict(os.environ)
    if args.gpus:
        env["HIP_VISIBLE_DEVICES"] = args.gpus
        env["CUDA_VISIBLE_DEVICES"] = args.gpus

    served_model_name = args.served_model_name or args.model
    cmd = [
        str(VLLM_PYTHON),
        "-m",
        "vllm.entrypoints.cli.main",
        "serve",
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
    if args.trust_remote_code:
        cmd.append("--trust-remote-code")
    cmd.extend(extra_args)

    _print("Launching the visible repo-local fork from external/vllm")
    _print(f"working_dir={VLLM_DIR}")
    _print(f"python={VLLM_PYTHON}")
    _print(f"backend_name={args.backend_name}")
    _print(f"model={args.model}")
    _print(f"endpoint=http://127.0.0.1:{args.port}/v1")
    _print("")
    _print("Register APXM after the server is reachable:")
    _print(
        "  dekk apxm backend add "
        f"{args.backend_name} --type onprem --protocol vllm --endpoint http://127.0.0.1:{args.port}/v1"
    )
    _print(f"  dekk apxm backend add-model {args.backend_name} {served_model_name}")
    _print(f"  dekk apxm backend test {args.backend_name}")
    _print("")

    return _run(cmd, cwd=VLLM_DIR, env=env)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="dekk apxm vllm",
        description="Operate the repo-local external/vllm fork.",
    )
    subparsers = parser.add_subparsers(dest="command", required=True)

    install = subparsers.add_parser("install", help="Install the repo-local vLLM fork")
    install.set_defaults(handler=lambda ns, extra: install_cmd(ns))

    serve = subparsers.add_parser("serve", help="Serve a model from the repo-local vLLM fork")
    serve.add_argument("model", help="Exact Hugging Face model id or local model path")
    serve.add_argument("--backend-name", default=DEFAULT_BACKEND_NAME, help="APXM backend name")
    serve.add_argument("--host", default=DEFAULT_HOST, help="Bind host for vLLM")
    serve.add_argument("--port", type=int, default=DEFAULT_PORT, help="Bind port for vLLM")
    serve.add_argument("--served-model-name", help="Override the model id exposed by the server")
    serve.add_argument(
        "--gpus",
        help="GPU list, applied to both HIP_VISIBLE_DEVICES and CUDA_VISIBLE_DEVICES",
    )
    serve.add_argument("--dtype", help="Model dtype passed through to vLLM")
    serve.add_argument("--max-model-len", type=int, help="Maximum model context length")
    serve.add_argument("--tensor-parallel-size", type=int, help="Tensor parallel size")
    serve.add_argument(
        "--gpu-memory-utilization",
        type=float,
        help="GPU memory utilization fraction",
    )
    serve.add_argument("--download-dir", help="Model download/cache directory")
    serve.add_argument("--tool-call-parser", help="Enable a specific tool-call parser")
    serve.add_argument(
        "--enable-auto-tool-choice",
        action="store_true",
        help="Enable auto tool choice support",
    )
    serve.add_argument(
        "--trust-remote-code",
        action="store_true",
        help="Pass --trust-remote-code to vLLM",
    )
    serve.set_defaults(handler=serve_cmd)

    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args, extra_args = parser.parse_known_args(argv)
    return args.handler(args, extra_args)


if __name__ == "__main__":
    raise SystemExit(main())
