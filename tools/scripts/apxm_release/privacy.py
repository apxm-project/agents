"""Fail-closed package and release privacy controls."""

from __future__ import annotations

import hashlib
import json
import shutil
import subprocess
import tomllib
from dataclasses import dataclass
from pathlib import Path
from urllib.parse import urlsplit

from apxm_release.constants import (
    CANONICAL_PYTHON_DISTRIBUTIONS,
    PRIVATE_PYTHON_CLASSIFIER,
    REGISTRY_MANIFEST_SCHEMA,
    REGISTRY_SIGNATURE_NAMESPACE,
    REPO_ROOT,
)
from apxm_release.util import load_toml, python_distribution_name, python_publish_enabled


@dataclass(frozen=True)
class PrivatePythonRegistry:
    """Verified destination authorized for one or more Python distributions."""

    repository_url: str
    package_names: tuple[str, ...]
    signer: str
    manifest_sha256: str


def _tracked_paths(*pathspecs: str) -> tuple[Path, ...]:
    process = subprocess.run(
        ["git", "ls-files", "-z", "--", *pathspecs],
        cwd=REPO_ROOT,
        check=False,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    if process.returncode != 0:
        detail = process.stderr.decode("utf-8", errors="replace").strip()
        raise RuntimeError(f"git ls-files failed: {detail or process.returncode}")
    paths = tuple(
        REPO_ROOT / Path(raw.decode("utf-8"))
        for raw in process.stdout.split(b"\0")
        if raw
    )
    return tuple(path for path in paths if path.is_file())


def _audit_cargo_manifest(path: Path) -> list[str]:
    manifest = load_toml(path)
    package = manifest.get("package")
    if not isinstance(package, dict):
        return []
    if package.get("publish") is not False:
        return [f"{path.relative_to(REPO_ROOT)}: package.publish must be false"]
    return []


def _audit_npm_manifest(path: Path) -> list[str]:
    relative = path.relative_to(REPO_ROOT)
    try:
        manifest = json.loads(path.read_text(encoding="utf-8"))
    except (json.JSONDecodeError, UnicodeDecodeError) as error:
        return [f"{relative}: invalid package manifest: {error}"]
    if not isinstance(manifest, dict):
        return [f"{relative}: package manifest must be an object"]

    publish_config = manifest.get("publishConfig")
    if manifest.get("private") is True:
        if publish_config is not None:
            return [f"{relative}: private package must not carry publishConfig"]
        return []
    return [
        f"{relative}: private must be true until signed private npm registry "
        "authority exists"
    ]


def _audit_python_manifest(path: Path) -> list[str]:
    relative = path.relative_to(REPO_ROOT)
    try:
        manifest = load_toml(path)
    except (OSError, tomllib.TOMLDecodeError) as error:
        return [f"{relative}: invalid Python package manifest: {error}"]
    project = manifest.get("project")
    if not isinstance(project, dict):
        return []

    violations: list[str] = []
    classifiers = project.get("classifiers")
    if not isinstance(classifiers, list) or PRIVATE_PYTHON_CLASSIFIER not in classifiers:
        violations.append(
            f"{relative}: classifiers must include {PRIVATE_PYTHON_CLASSIFIER!r}"
        )
    release = manifest.get("tool", {}).get("apxm", {}).get("release", {})
    if not isinstance(release, dict) or not isinstance(release.get("publish"), bool):
        violations.append(f"{relative}: tool.apxm.release.publish must be explicit")
    name = project.get("name")
    if release.get("publish") is True and name not in CANONICAL_PYTHON_DISTRIBUTIONS:
        violations.append(f"{relative}: {name!r} is not a canonical Python distribution")
    return violations


def _audit_release_commands() -> list[str]:
    paths = list(
        _tracked_paths(
            ".dekk.toml",
            "README.md",
            "RELEASING.md",
            ".github/workflows/*",
            "tools/scripts/release.py",
            "tools/scripts/apxm_release/cli.py",
            "tools/scripts/apxm_release/dist.py",
            "tools/scripts/apxm_release/publish.py",
        )
    )
    forbidden = {
        "--access public": "public npm access",
        "npm publish": "npm publishing without a signed registry manifest",
        "cargo publish": "Cargo publishing without a signed registry manifest",
        "pypi.org": "public Python package index",
        "npmjs.com": "public npm registry",
        "crates.io": "public Rust registry",
        "dekk apxm": "non-owner Dekk namespace",
    }
    violations: list[str] = []
    for path in paths:
        text = path.read_text(encoding="utf-8").lower()
        relative = path.relative_to(REPO_ROOT)
        for needle, label in forbidden.items():
            if needle in text:
                violations.append(f"{relative}: contains {label}")
    return violations


def audit_release_privacy() -> tuple[str, ...]:
    """Enumerate every tracked package manifest and release command surface."""

    violations: list[str] = []
    for path in _tracked_paths("Cargo.toml", "**/Cargo.toml"):
        violations.extend(_audit_cargo_manifest(path))
    for path in _tracked_paths("package.json", "**/package.json"):
        violations.extend(_audit_npm_manifest(path))
    for path in _tracked_paths(
        "pyproject.toml",
        "**/pyproject.toml",
        "setup.py",
        "**/setup.py",
        "setup.cfg",
        "**/setup.cfg",
    ):
        if path.name != "pyproject.toml":
            violations.append(
                f"{path.relative_to(REPO_ROOT)}: unsupported Python packaging surface"
            )
            continue
        violations.extend(_audit_python_manifest(path))
    violations.extend(_audit_release_commands())
    return tuple(sorted(set(violations)))


def _require_file(path: Path, label: str) -> None:
    if not path.is_file():
        raise ValueError(f"{label} is not a file: {path}")


def _verify_signature(
    manifest: bytes,
    *,
    signature_path: Path,
    allowed_signers_path: Path,
    signer: str,
) -> None:
    if shutil.which("ssh-keygen") is None:
        raise ValueError("ssh-keygen is required to verify registry authorization")
    process = subprocess.run(
        [
            "ssh-keygen",
            "-Y",
            "verify",
            "-f",
            str(allowed_signers_path),
            "-I",
            signer,
            "-n",
            REGISTRY_SIGNATURE_NAMESPACE,
            "-s",
            str(signature_path),
        ],
        input=manifest,
        check=False,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    if process.returncode != 0:
        detail = process.stderr.decode("utf-8", errors="replace").strip()
        raise ValueError(f"registry manifest signature verification failed: {detail}")


def _private_repository_url(value: object) -> str:
    if not isinstance(value, str):
        raise ValueError("repository_url must be a string")
    parsed = urlsplit(value)
    if parsed.scheme != "https" or not parsed.hostname:
        raise ValueError("repository_url must be an absolute HTTPS URL")
    if parsed.username or parsed.password or parsed.query or parsed.fragment:
        raise ValueError("repository_url must not contain credentials, query, or fragment")
    hostname = parsed.hostname.lower().rstrip(".")
    if hostname == "pypi.org" or hostname.endswith(".pypi.org"):
        raise ValueError("public Python package indexes are forbidden")
    return value


def load_private_python_registry(
    manifest_path: Path,
    signature_path: Path,
    allowed_signers_path: Path,
    signer: str,
) -> PrivatePythonRegistry:
    """Verify and parse an exact release-controller-authorized registry manifest."""

    _require_file(manifest_path, "registry manifest")
    _require_file(signature_path, "registry signature")
    _require_file(allowed_signers_path, "allowed signers")
    if not signer or signer.strip() != signer or any(character.isspace() for character in signer):
        raise ValueError("signer must be one non-whitespace identity")

    manifest_bytes = manifest_path.read_bytes()
    _verify_signature(
        manifest_bytes,
        signature_path=signature_path,
        allowed_signers_path=allowed_signers_path,
        signer=signer,
    )
    try:
        payload = json.loads(manifest_bytes)
    except (json.JSONDecodeError, UnicodeDecodeError) as error:
        raise ValueError(f"registry manifest is not valid UTF-8 JSON: {error}") from error
    if not isinstance(payload, dict):
        raise ValueError("registry manifest must be an object")
    expected_keys = {
        "schema_version",
        "ecosystem",
        "visibility",
        "repository_url",
        "package_names",
    }
    if set(payload) != expected_keys:
        raise ValueError("registry manifest fields do not match the strict schema")
    if payload["schema_version"] != REGISTRY_MANIFEST_SCHEMA:
        raise ValueError("registry manifest schema_version is unsupported")
    if payload["ecosystem"] != "python":
        raise ValueError("registry manifest ecosystem must be python")
    if payload["visibility"] != "private":
        raise ValueError("registry manifest visibility must be private")
    package_names = payload["package_names"]
    if (
        not isinstance(package_names, list)
        or not package_names
        or any(not isinstance(name, str) or not name for name in package_names)
        or len(package_names) != len(set(package_names))
    ):
        raise ValueError("registry manifest package_names must be unique non-empty strings")
    return PrivatePythonRegistry(
        repository_url=_private_repository_url(payload["repository_url"]),
        package_names=tuple(package_names),
        signer=signer,
        manifest_sha256=hashlib.sha256(manifest_bytes).hexdigest(),
    )


def require_publishable_python_distribution(registry: PrivatePythonRegistry) -> str:
    """Reject prototype/internal Python packaging before any upload process starts."""

    distribution = python_distribution_name()
    if not python_publish_enabled():
        raise ValueError(f"Python distribution {distribution!r} is marked non-publishable")
    if distribution not in CANONICAL_PYTHON_DISTRIBUTIONS:
        raise ValueError(f"Python distribution {distribution!r} is not canonical")
    if distribution not in registry.package_names:
        raise ValueError(
            f"registry manifest does not authorize Python distribution {distribution!r}"
        )
    return distribution
