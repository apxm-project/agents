#!/usr/bin/env python3
"""Emit a deterministic exact startup-input artifact for the reference host."""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import os
import platform
import re
import subprocess
import sys
from pathlib import Path
from typing import Any


REPOSITORY_ROOT = Path(__file__).resolve().parents[2]
VALIDATOR_PATH = REPOSITORY_ROOT / "contracts" / "tools" / "validate_owner_descriptor.py"
RELEASE_MANIFEST_PATH = (
    REPOSITORY_ROOT
    / "contracts"
    / "reference-host"
    / "manifests"
    / "apxm.reference-host-release-manifest.v1.json"
)
DEFAULT_OUTPUT_PATH = (
    REPOSITORY_ROOT
    / ".apxm"
    / "reference-host"
    / "startup-inputs"
    / "apxm.reference-host-startup-input.v1.json"
)
STARTUP_INPUT_SCHEMA = "apxm.reference-host-startup-input.v1"
OWNER_EXECUTABLE = "apxm-reference-host"
OWNER_EXECUTABLE_PATH = "crates/tools/cli/src/bin/reference_host.rs"
TRANSPORT_PROTOCOL = "jsonl-stdin-stdout"
HEX40 = re.compile(r"^[0-9a-f]{40}$")
HEX64 = re.compile(r"^sha256:[0-9a-f]{64}$")
FAIL_CLOSED_ON = ["missing", "placeholder", "dirty", "mismatched", "implicit-default"]


def load_validator_module():
    spec = importlib.util.spec_from_file_location("validate_owner_descriptor", VALIDATOR_PATH)
    if spec is None or spec.loader is None:
        raise RuntimeError("unable to load validate_owner_descriptor.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def file_digest(path: Path) -> str:
    return "sha256:" + hashlib.sha256(path.read_bytes()).hexdigest()


def git_stdout(*args: str) -> str:
    env = os.environ.copy()
    if platform.system() == "Darwin":
        env.pop("DYLD_LIBRARY_PATH", None)
        env.pop("DYLD_FALLBACK_LIBRARY_PATH", None)
        env.pop("LD_LIBRARY_PATH", None)
    result = subprocess.run(
        ["git", *args],
        cwd=REPOSITORY_ROOT,
        check=False,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        env=env,
    )
    if result.returncode != 0:
        detail = result.stderr.strip() or result.stdout.strip() or str(result.returncode)
        raise RuntimeError(f"git {' '.join(args)} failed: {detail}")
    return result.stdout.strip()


def is_placeholder_digest(digest: str) -> bool:
    hex_value = digest.removeprefix("sha256:")
    return len(set(hex_value)) == 1


def validate_revision(owner_revision: str) -> None:
    if not owner_revision:
        raise ValueError("owner_revision must be explicit; implicit/default values are rejected")
    if HEX40.fullmatch(owner_revision) is None:
        raise ValueError("owner_revision must be a lowercase 40-hex git revision")


def validate_exact_digest(name: str, value: str) -> None:
    if not value:
        raise ValueError(f"{name} must be explicit; implicit/default values are rejected")
    if HEX64.fullmatch(value) is None:
        raise ValueError(f"{name} must be a lowercase sha256:<64 hex> digest")
    if is_placeholder_digest(value):
        raise ValueError(f"{name} must not be a placeholder digest")


def write_json(path: Path, payload: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def build_startup_input(
    *,
    owner_revision: str,
    release_digest: str,
    port_bindings_digest: str,
    resource_ceiling_digest: str,
    output_path: Path = DEFAULT_OUTPUT_PATH,
) -> dict[str, Any]:
    validate_revision(owner_revision)
    validate_exact_digest("release_digest", release_digest)
    validate_exact_digest("port_bindings_digest", port_bindings_digest)
    validate_exact_digest("resource_ceiling_digest", resource_ceiling_digest)

    checkout_revision = git_stdout("rev-parse", "HEAD")
    if owner_revision != checkout_revision:
        raise RuntimeError(
            "owner revision mismatch: "
            f"expected {owner_revision}, checkout is {checkout_revision}"
        )
    status = git_stdout("status", "--porcelain", "--ignored=matching")
    if status:
        raise RuntimeError(
            "owner checkout is dirty; exact reference-host startup input requires committed bytes"
        )

    validator = load_validator_module()
    descriptor_exact_checksum = validator.descriptor_exact_checksum()
    payload = {
        "schema_version": STARTUP_INPUT_SCHEMA,
        "semantic_owner": "agents",
        "owner_executable": OWNER_EXECUTABLE,
        "owner_executable_path": OWNER_EXECUTABLE_PATH,
        "transport_protocol": TRANSPORT_PROTOCOL,
        "reference_host_release_manifest": {
            "path": str(RELEASE_MANIFEST_PATH.resolve()),
            "digest": file_digest(RELEASE_MANIFEST_PATH),
        },
        "release_digest": release_digest,
        "port_bindings_digest": port_bindings_digest,
        "resource_ceiling_digest": resource_ceiling_digest,
        "provenance": {
            "owner_revision": owner_revision,
            "descriptor_semantic_digest": validator.REFERENCE_HOST_DESCRIPTOR[
                "descriptor_semantic_digest"
            ],
            "descriptor_exact_checksum": descriptor_exact_checksum,
            "dirty": False,
        },
        "fail_closed_on": FAIL_CLOSED_ON,
    }
    write_json(output_path, payload)
    return payload


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--owner-revision", required=True)
    parser.add_argument("--release-digest", required=True)
    parser.add_argument("--port-bindings-digest", required=True)
    parser.add_argument("--resource-ceiling-digest", required=True)
    parser.add_argument("--output-path", type=Path, default=DEFAULT_OUTPUT_PATH)
    return parser.parse_args(argv)


def main(argv: list[str]) -> int:
    args = parse_args(argv)
    payload = build_startup_input(
        owner_revision=args.owner_revision,
        release_digest=args.release_digest,
        port_bindings_digest=args.port_bindings_digest,
        resource_ceiling_digest=args.resource_ceiling_digest,
        output_path=args.output_path,
    )
    print(json.dumps(payload, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
