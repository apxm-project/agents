#!/usr/bin/env python3
"""Generate and validate the Agents-owned Linux/arm64 reference-host image recipe."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import subprocess
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[2]
DOCKERFILE_PATH = "deploy/reference-host/Dockerfile"
DOCKERIGNORE_PATH = ".dockerignore"
LOCK_PATH = "Cargo.lock"
MANIFEST_PATH = ROOT / "deploy/reference-host/image-manifest.v1.json"
SIDECAR_PATH = ROOT / "deploy/reference-host/image-manifest.v1.sha256"
REFERENCE_HOST_RELEASE_PATH = (
    "contracts/reference-host/manifests/apxm.reference-host-release-manifest.v1.json"
)
HARNESS_PATH = "crates/tools/cli/tests/reference_host_jsonl.rs"
LIFECYCLE_VECTOR_PATH = (
    "contracts/reference-host/vectors/apxm.reference-host.lifecycle-parity.v1.json"
)
SOURCE_REVISION = "140ce5b16b4bc4a078800b52fffbfc8004169ee1"
REVIEWED_CARRIER_REVISION = "8f767541413d0905d638b288a797c5825ceb1c76"
BUILD_IMAGE = (
    "rust:1.89-bookworm@"
    "sha256:948f9b08a66e7fe01b03a98ef1c7568292e07ec2e4fe90d88c07bb14563c84ff"
)
RUNTIME_IMAGE = (
    "debian:bookworm-slim@"
    "sha256:abd67ffcfa541b485a3dff59865ab629aa048a6c613e639d36e7456b0b229241"
)


class ImageManifestError(ValueError):
    """The reference-host image recipe is not the exact owner publication."""


def digest_bytes(value: bytes) -> str:
    return "sha256:" + hashlib.sha256(value).hexdigest()


def canonical(value: dict[str, Any]) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":")).encode("utf-8")


def json_bytes(value: dict[str, Any]) -> bytes:
    return (json.dumps(value, indent=2, sort_keys=True) + "\n").encode("utf-8")


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ImageManifestError(message)


def git(*args: str) -> bytes:
    environment = os.environ.copy()
    environment.pop("DYLD_LIBRARY_PATH", None)
    environment.pop("DYLD_FALLBACK_LIBRARY_PATH", None)
    result = subprocess.run(
        ["git", "-C", str(ROOT), *args],
        capture_output=True,
        check=False,
        env=environment,
    )
    if result.returncode:
        detail = result.stderr.decode("utf-8", errors="replace").strip()
        raise ImageManifestError(f"git {' '.join(args)} failed: {detail}")
    return result.stdout


def git_text(*args: str) -> str:
    return git(*args).decode("utf-8").strip()


def git_file(revision: str, path: str) -> bytes:
    return git("show", f"{revision}:{path}")


def committed_inputs() -> dict[str, str]:
    require(
        git_text("rev-parse", f"{SOURCE_REVISION}^{{commit}}") == SOURCE_REVISION,
        "source revision is not committed",
    )
    require(
        git_text("rev-parse", f"{REVIEWED_CARRIER_REVISION}^{{commit}}")
        == REVIEWED_CARRIER_REVISION,
        "reviewed carrier revision is not committed",
    )
    require(
        git("merge-base", "--is-ancestor", SOURCE_REVISION, REVIEWED_CARRIER_REVISION)
        == b"",
        "reviewed carrier is not a direct descendant cohort of source",
    )
    source_release = git_file(SOURCE_REVISION, REFERENCE_HOST_RELEASE_PATH)
    carrier_release = git_file(REVIEWED_CARRIER_REVISION, REFERENCE_HOST_RELEASE_PATH)
    require(
        source_release == carrier_release,
        "reviewed carrier changed reference-host release-manifest bytes",
    )
    return {
        "reference_host_release_manifest_digest": digest_bytes(source_release),
        "cargo_lock_digest": digest_bytes(git_file(SOURCE_REVISION, LOCK_PATH)),
        "harness_digest": digest_bytes(git_file(SOURCE_REVISION, HARNESS_PATH)),
        "lifecycle_vector_digest": digest_bytes(
            git_file(SOURCE_REVISION, LIFECYCLE_VECTOR_PATH)
        ),
    }


def build_manifest() -> dict[str, Any]:
    facts = committed_inputs()
    dockerfile = (ROOT / DOCKERFILE_PATH).read_bytes()
    dockerignore = (ROOT / DOCKERIGNORE_PATH).read_bytes()
    lock = (ROOT / LOCK_PATH).read_bytes()
    require(
        digest_bytes(lock) == facts["cargo_lock_digest"],
        "working Cargo.lock differs from exact source bytes",
    )
    rendered = dockerfile.decode("utf-8")
    for marker in (
        "# syntax=docker/dockerfile:1.7@sha256:a57df69d0ea827fb7266491f2813635de6f17269be881f696fbfdf2d83dda33e",
        "FROM --platform=linux/arm64",
        'test "$TARGETPLATFORM" = "linux/arm64"',
        "cargo test --locked -p apxm-cli --test reference_host_jsonl --release",
        "cargo build --locked -p apxm-cli --bin apxm-reference-host --release",
        "SOURCE_DATE_EPOCH=1786322092",
        'touch --date="@$SOURCE_DATE_EPOCH" target/release/apxm-reference-host',
        "7f454c460201010000000000000000000300b700",
        f"org.opencontainers.image.revision=\"$APXM_SOURCE_REVISION\"",
        f'io.apxm.reviewed-carrier-revision="$APXM_REVIEWED_CARRIER_REVISION"',
    ):
        require(marker in rendered, f"Dockerfile is missing exact marker: {marker}")
    manifest: dict[str, Any] = {
        "schema_version": "apxm.reference-host-image-manifest.v1",
        "semantic_owner": "agents",
        "image_family": "apxm-reference-host",
        "source_revision": SOURCE_REVISION,
        "reviewed_carrier_revision": REVIEWED_CARRIER_REVISION,
        "reference_host_release_manifest": {
            "path": REFERENCE_HOST_RELEASE_PATH,
            "digest": facts["reference_host_release_manifest_digest"],
        },
        "build_recipe": {
            "dockerfile_path": DOCKERFILE_PATH,
            "dockerfile_digest": digest_bytes(dockerfile),
            "dockerignore_path": DOCKERIGNORE_PATH,
            "dockerignore_digest": digest_bytes(dockerignore),
            "cargo_lock_path": LOCK_PATH,
            "cargo_lock_digest": facts["cargo_lock_digest"],
            "build_image": BUILD_IMAGE,
            "runtime_image": RUNTIME_IMAGE,
            "command": (
                "docker build --platform=linux/arm64 --provenance=false "
                "--build-arg SOURCE_DATE_EPOCH=1786322092 "
                "--file deploy/reference-host/Dockerfile "
                "--tag apxm-reference-host:<carrier> ."
            ),
        },
        "platform": {
            "os": "linux",
            "architecture": "arm64",
            "elf_machine": "AArch64",
            "rust_target": "aarch64-unknown-linux-gnu",
        },
        "lifecycle_gate": {
            "command": (
                "cargo test --locked -p apxm-cli --test reference_host_jsonl --release"
            ),
            "harness_path": HARNESS_PATH,
            "harness_digest": facts["harness_digest"],
            "vector_path": LIFECYCLE_VECTOR_PATH,
            "vector_digest": facts["lifecycle_vector_digest"],
        },
        "runtime": {
            "entrypoint": ["/usr/local/bin/apxm-reference-host"],
            "user": "65532:65532",
            "source_mount": "absent",
        },
    }
    manifest["manifest_digest"] = digest_bytes(canonical(manifest))
    return manifest


def validate_payload(manifest: dict[str, Any]) -> dict[str, str]:
    expected = build_manifest()
    require(manifest == expected, "reference-host image manifest drifted from owner inputs")
    expected_bytes = json_bytes(expected)
    require(MANIFEST_PATH.read_bytes() == expected_bytes, "image manifest bytes are not canonical")
    sidecar = SIDECAR_PATH.read_text(encoding="utf-8").splitlines()
    require(len(sidecar) == 2, "image manifest sidecar shape drifted")
    require(
        sidecar[0]
        == f"{digest_bytes(expected_bytes)}  deploy/reference-host/image-manifest.v1.json",
        "image manifest sidecar digest is stale",
    )
    require(sidecar[1] == "digest_scope: exact repository bytes", "sidecar scope drifted")
    return {
        "manifest_digest": expected["manifest_digest"],
        "exact_bytes_digest": digest_bytes(expected_bytes),
    }


def write_manifest() -> dict[str, str]:
    manifest = build_manifest()
    encoded = json_bytes(manifest)
    MANIFEST_PATH.parent.mkdir(parents=True, exist_ok=True)
    MANIFEST_PATH.write_bytes(encoded)
    SIDECAR_PATH.write_text(
        f"{digest_bytes(encoded)}  deploy/reference-host/image-manifest.v1.json\n"
        "digest_scope: exact repository bytes\n",
        encoding="utf-8",
    )
    return validate_payload(manifest)


def validate(*, require_clean: bool = True) -> dict[str, str]:
    if require_clean:
        require(
            not git_text("status", "--porcelain=v1", "--untracked-files=all"),
            "owner checkout is dirty",
        )
    try:
        manifest = json.loads(MANIFEST_PATH.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise ImageManifestError(f"cannot read image manifest: {error}") from error
    require(isinstance(manifest, dict), "image manifest must be an object")
    return validate_payload(manifest)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    action = parser.add_mutually_exclusive_group(required=True)
    action.add_argument("--write", action="store_true")
    action.add_argument("--check", action="store_true")
    parser.add_argument("--allow-dirty", action="store_true")
    args = parser.parse_args()
    try:
        result = write_manifest() if args.write else validate(require_clean=not args.allow_dirty)
    except (ImageManifestError, KeyError) as error:
        print(f"FAIL: {error}")
        return 1
    print("PASS: Agents reference-host Linux/arm64 image recipe")
    print(f"manifest_digest: {result['manifest_digest']}")
    print(f"exact_bytes_digest: {result['exact_bytes_digest']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
