"""Qualify an immutable APXM owner release.

The gate is deliberately owner-local.  It runs the declared Compilation and
Runtime protocol suites, validates the product-neutral release descriptors,
and compares every available digest with the exact bytes it names.  A source
checkout is not a publishable runtime release merely because it builds.

The ``generate`` mode writes descriptors only when the caller supplies a real
artifact.  It computes digests from bytes; it never creates placeholder or
sentinel identities.
"""

from __future__ import annotations

import argparse
from dataclasses import dataclass, field
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
from typing import Any, Iterable, Mapping, Sequence


REPOSITORY_ROOT = Path(__file__).resolve().parents[2]

SOURCE_DESCRIPTOR_REL = Path("deploy/services/source-revision.v1.json")
OWNER_DESCRIPTOR_REL = Path("contracts/descriptors/apxm.agents-owner-descriptor.v1.json")
OWNER_DESCRIPTOR_SIDECAR_REL = Path(
    "contracts/descriptors/apxm.agents-owner-descriptor.v1.sha256"
)
RELEASE_MANIFEST_REL = Path(
    "contracts/services/manifests/apxm.agents-service-release-manifest.v1.json"
)

SOURCE_DESCRIPTOR_SCHEMA = "apxm.agents-source-revision.v1"
OWNER_DESCRIPTOR_SCHEMA = "apxm.agents-owner-descriptor.v1"
RELEASE_MANIFEST_SCHEMA = "apxm.agents-service-release-manifest.v1"
COMPILATION_PROTOCOL_VERSION = "apxm.compilation.protocol/1"
RUNTIME_PROTOCOL_VERSION = "apxm.runtime.protocol/1"
ARTIFACT_KIND = "compilation-runtime-services"

SERVICE_ARTIFACTS = (
    ("compilation-service", "apxm-compilation-service"),
    ("runtime-service", "apxm-runtime-service"),
)

HEX40 = re.compile(r"^[0-9a-f]{40}$")
DIGEST = re.compile(r"^sha256:[0-9a-f]{64}$")
SAFE_RELATIVE_PATH = re.compile(r"^(?!/)(?!.*(?:^|/)\.\.(?:/|$)).+$")

PROTOCOL_GATES = (
    "test-compilation-protocol",
    "test-compilation-service",
    "test-runtime-protocol",
    "test-runtime-service",
)
OWNER_GATES = ("check", "test", *PROTOCOL_GATES)


@dataclass(frozen=True)
class Diagnostic:
    code: str
    message: str
    remediation: str

    def render(self) -> str:
        return f"[{self.code}] {self.message} Remediation: {self.remediation}"


@dataclass
class Qualification:
    diagnostics: list[Diagnostic] = field(default_factory=list)
    gates: list[dict[str, Any]] = field(default_factory=list)
    artifacts: dict[str, Path] = field(default_factory=dict)
    artifact_digests: dict[str, str] = field(default_factory=dict)

    @property
    def ok(self) -> bool:
        return not self.diagnostics


def _digest_bytes(value: bytes) -> str:
    return f"sha256:{hashlib.sha256(value).hexdigest()}"


def _digest_file(path: Path) -> str:
    return _digest_bytes(path.read_bytes())


def _canonical_json(value: Mapping[str, Any]) -> bytes:
    return (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode(
        "utf-8"
    )


def _load_json(
    root: Path, path: Path, diagnostics: list[Diagnostic], label: str
) -> dict[str, Any] | None:
    resolved = _resolve_regular_file(root, path)
    if resolved is None:
        if path.is_symlink() or path.exists():
            diagnostics.append(
                Diagnostic(
                    "non-regular-descriptor",
                    f"{label} is not a regular file inside the owner checkout: {path}",
                    "publish the exact descriptor bytes in the owner checkout; symlinks and external files are rejected",
                )
            )
        else:
            diagnostics.append(
                Diagnostic(
                    "missing-descriptor",
                    f"{label} is missing: {path}",
                    "publish the owner descriptor/manifest generated from the selected source revision",
                )
            )
        return None
    try:
        value = json.loads(resolved.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as exc:
        diagnostics.append(
            Diagnostic(
                "invalid-json",
                f"{label} cannot be read as UTF-8 JSON ({exc})",
                "regenerate the descriptor from the owner checkout and commit the exact bytes",
            )
        )
        return None
    if not isinstance(value, dict):
        diagnostics.append(
            Diagnostic(
                "invalid-schema",
                f"{label} root must be a JSON object",
                "regenerate the descriptor with the release-descriptor tooling",
            )
        )
        return None
    return value


def _require_exact_shape(
    value: Mapping[str, Any],
    *,
    label: str,
    required: Iterable[str],
    optional: Iterable[str],
    diagnostics: list[Diagnostic],
) -> bool:
    required_set = set(required)
    allowed = required_set | set(optional)
    missing = sorted(key for key in required_set if key not in value)
    unknown = sorted(key for key in value if key not in allowed)
    valid = True
    if missing:
        diagnostics.append(
            Diagnostic(
                "invalid-schema",
                f"{label} is missing required field(s): {', '.join(missing)}",
                "regenerate the descriptor; unknown and omitted release identity fields are rejected",
            )
        )
        valid = False
    if unknown:
        diagnostics.append(
            Diagnostic(
                "invalid-schema",
                f"{label} contains unknown field(s): {', '.join(unknown)}",
                "remove fields outside the product-neutral owner release schema",
            )
        )
        valid = False
    return valid


def _require_string(
    value: Mapping[str, Any],
    key: str,
    *,
    label: str,
    diagnostics: list[Diagnostic],
    pattern: re.Pattern[str] | None = None,
    expected: str | None = None,
) -> bool:
    raw = value.get(key)
    valid = isinstance(raw, str) and bool(raw.strip())
    if valid and pattern is not None:
        valid = bool(pattern.fullmatch(raw))
    if valid and expected is not None:
        valid = raw == expected
    if not valid:
        expectation = f" equal {expected!r}" if expected is not None else " match the declared schema"
        diagnostics.append(
            Diagnostic(
                "invalid-schema",
                f"{label}.{key} is invalid; it must be a non-empty string{expectation}",
                "regenerate the descriptor with the owner release tooling",
            )
        )
    return valid


def _git_text(root: Path, *args: str) -> str | None:
    git_env = os.environ.copy()
    if sys.platform == "darwin":
        # Dekk's conda environment ships libiconv, while Homebrew's git links
        # against its own ABI. Keep the Dekk environment for Python and Rust,
        # but do not let it interpose on the Git subprocess used for identity.
        git_env.pop("DYLD_LIBRARY_PATH", None)
        git_env.pop("DYLD_FALLBACK_LIBRARY_PATH", None)
    completed = subprocess.run(
        ["git", "-C", str(root), *args],
        check=False,
        capture_output=True,
        text=True,
        env=git_env,
    )
    if completed.returncode != 0:
        return None
    return completed.stdout.strip()


def _git_revision(root: Path) -> str | None:
    revision = _git_text(root, "rev-parse", "HEAD")
    return revision if revision and HEX40.fullmatch(revision) else None


def _git_is_ancestor(root: Path, ancestor: str, descendant: str) -> bool:
    git_env = os.environ.copy()
    if sys.platform == "darwin":
        git_env.pop("DYLD_LIBRARY_PATH", None)
        git_env.pop("DYLD_FALLBACK_LIBRARY_PATH", None)
    completed = subprocess.run(
        ["git", "-C", str(root), "merge-base", "--is-ancestor", ancestor, descendant],
        check=False,
        capture_output=True,
        env=git_env,
    )
    return completed.returncode == 0


def _is_clean(root: Path) -> bool:
    return _git_text(root, "status", "--porcelain", "--untracked-files=all") == ""


def _resolve_regular_file(root: Path, candidate: Path) -> Path | None:
    """Resolve a release input only when it stays inside the owner checkout."""

    if candidate.is_symlink() or not candidate.is_file():
        return None
    try:
        resolved = candidate.resolve(strict=True)
        resolved.relative_to(root.resolve())
    except (OSError, ValueError):
        return None
    return resolved


def _rooted_path(root: Path, raw: str | Path) -> Path:
    candidate = Path(raw).expanduser()
    return candidate if candidate.is_absolute() else root / candidate


def _external_service_binding(name: str) -> Path | None:
    """Return an exact externally materialized service artifact, if bound."""

    environment_name = (
        "CLIC_APXM_COMPILATION_SERVICE"
        if name == "compilation-service"
        else "CLIC_APXM_RUNTIME_SERVICE"
    )
    value = os.environ.get(environment_name, "").strip()
    match = re.fullmatch(r"(.+)@(sha256:[0-9a-f]{64})", value)
    if match is None:
        return None
    artifact = Path(match.group(1)).expanduser()
    if artifact.is_symlink() or not artifact.is_file() or not os.access(artifact, os.X_OK):
        return None
    try:
        if _digest_file(artifact) != match.group(2):
            return None
    except OSError:
        return None
    return artifact.resolve()


def _validate_source_descriptor(
    root: Path, value: Mapping[str, Any] | None, diagnostics: list[Diagnostic]
) -> str | None:
    if value is None:
        return None
    valid = _require_exact_shape(
        value,
        label="source descriptor",
        required=("schema_version", "semantic_owner", "selection", "source_revision"),
        optional=(),
        diagnostics=diagnostics,
    )
    valid &= _require_string(
        value,
        "schema_version",
        label="source descriptor",
        diagnostics=diagnostics,
        expected=SOURCE_DESCRIPTOR_SCHEMA,
    )
    valid &= _require_string(
        value,
        "semantic_owner",
        label="source descriptor",
        diagnostics=diagnostics,
        expected="agents",
    )
    valid &= _require_string(
        value,
        "selection",
        label="source descriptor",
        diagnostics=diagnostics,
        expected="immutable-source-cohort",
    )
    valid &= _require_string(
        value,
        "source_revision",
        label="source descriptor",
        diagnostics=diagnostics,
        pattern=HEX40,
    )
    revision = value.get("source_revision") if valid else None
    if valid and _git_text(root, "rev-parse", f"{revision}^{{commit}}") != revision:
        diagnostics.append(
            Diagnostic(
                "unresolvable-source",
                f"source descriptor names revision {revision}, which is not present in the owner checkout",
                "fetch or publish the exact source cohort before generating a release",
            )
        )
        return None
    if valid and not _git_is_ancestor(root, revision or "", "HEAD"):
        diagnostics.append(
            Diagnostic(
                "source-not-ancestor",
                f"source descriptor revision {revision} is not an ancestor of the checkout HEAD",
                "publish from a checkout carrying the declared immutable source cohort",
            )
        )
    return revision if valid else None


def _validate_owner_descriptor(
    value: Mapping[str, Any] | None,
    *,
    source_revision: str | None,
    source_descriptor_digest: str | None,
    diagnostics: list[Diagnostic],
) -> None:
    if value is None:
        return
    valid = _require_exact_shape(
        value,
        label="owner descriptor",
        required=(
            "schema_version",
            "semantic_owner",
            "source_revision",
            "source_descriptor_digest",
            "artifact_kind",
            "compilation_protocol",
            "runtime_protocol",
        ),
        optional=(),
        diagnostics=diagnostics,
    )
    valid &= _require_string(
        value, "schema_version", label="owner descriptor", diagnostics=diagnostics, expected=OWNER_DESCRIPTOR_SCHEMA
    )
    valid &= _require_string(
        value, "semantic_owner", label="owner descriptor", diagnostics=diagnostics, expected="agents"
    )
    valid &= _require_string(
        value, "source_revision", label="owner descriptor", diagnostics=diagnostics, pattern=HEX40
    )
    valid &= _require_string(
        value,
        "source_descriptor_digest",
        label="owner descriptor",
        diagnostics=diagnostics,
        pattern=DIGEST,
    )
    valid &= _require_string(
        value, "artifact_kind", label="owner descriptor", diagnostics=diagnostics, expected=ARTIFACT_KIND
    )
    valid &= _require_string(
        value,
        "compilation_protocol",
        label="owner descriptor",
        diagnostics=diagnostics,
        expected=COMPILATION_PROTOCOL_VERSION,
    )
    valid &= _require_string(
        value,
        "runtime_protocol",
        label="owner descriptor",
        diagnostics=diagnostics,
        expected=RUNTIME_PROTOCOL_VERSION,
    )
    if valid and source_revision != value.get("source_revision"):
        diagnostics.append(
            Diagnostic(
                "source-digest-drift",
                "owner descriptor source_revision does not match the source descriptor",
                "regenerate both descriptors from one immutable source cohort",
            )
        )
    if valid and source_descriptor_digest != value.get("source_descriptor_digest"):
        diagnostics.append(
            Diagnostic(
                "source-digest-drift",
                "owner descriptor source_descriptor_digest does not match the exact source descriptor bytes",
                "regenerate the owner descriptor; do not hand-edit digest fields",
            )
        )


def _validate_release_manifest(
    root: Path,
    value: Mapping[str, Any] | None,
    *,
    source_revision: str | None,
    owner_descriptor_digest: str | None,
    artifacts: Mapping[str, Path],
    artifact_digests: Mapping[str, str],
    diagnostics: list[Diagnostic],
) -> None:
    if value is None:
        return
    valid = _require_exact_shape(
        value,
        label="release manifest",
        required=(
            "schema_version",
            "semantic_owner",
            "source_revision",
            "artifact_kind",
            "services",
            "owner_descriptor_digest",
            "integrity_algorithm",
        ),
        optional=(),
        diagnostics=diagnostics,
    )
    valid &= _require_string(
        value, "schema_version", label="release manifest", diagnostics=diagnostics, expected=RELEASE_MANIFEST_SCHEMA
    )
    valid &= _require_string(
        value, "semantic_owner", label="release manifest", diagnostics=diagnostics, expected="agents"
    )
    valid &= _require_string(
        value, "source_revision", label="release manifest", diagnostics=diagnostics, pattern=HEX40
    )
    valid &= _require_string(
        value, "artifact_kind", label="release manifest", diagnostics=diagnostics, expected=ARTIFACT_KIND
    )
    valid &= _require_string(
        value, "integrity_algorithm", label="release manifest", diagnostics=diagnostics, expected="sha256"
    )
    valid &= _require_string(
        value,
        "owner_descriptor_digest",
        label="release manifest",
        diagnostics=diagnostics,
        pattern=DIGEST,
    )
    if valid and owner_descriptor_digest != value.get("owner_descriptor_digest"):
        diagnostics.append(
            Diagnostic(
                "descriptor-digest-drift",
                "release manifest owner_descriptor_digest does not match the owner descriptor bytes",
                "regenerate the release manifest from the exact descriptor cohort",
            )
        )
    if valid and source_revision != value.get("source_revision"):
        diagnostics.append(
            Diagnostic(
                "source-digest-drift",
                "release manifest source_revision does not match the source descriptor",
                "regenerate the release manifest from one immutable source cohort",
            )
        )
    services = value.get("services")
    if not isinstance(services, list):
        diagnostics.append(
            Diagnostic(
                "invalid-schema",
                "release manifest.services must be an array",
                "regenerate the manifest with exactly one compilation-service and one runtime-service entry",
            )
        )
        return
    expected_names = {name for name, _ in SERVICE_ARTIFACTS}
    seen_names: set[str] = set()
    for index, service in enumerate(services):
        label = f"release manifest.services[{index}]"
        if not isinstance(service, dict):
            diagnostics.append(
                Diagnostic(
                    "invalid-schema",
                    f"{label} must be an object",
                    "regenerate the manifest with named service artifact entries",
                )
            )
            continue
        entry_valid = _require_exact_shape(
            service,
            label=label,
            required=("name", "path", "digest"),
            optional=(),
            diagnostics=diagnostics,
        )
        entry_valid &= _require_string(
            service, "name", label=label, diagnostics=diagnostics
        )
        entry_valid &= _require_string(
            service, "path", label=label, diagnostics=diagnostics, pattern=SAFE_RELATIVE_PATH
        )
        entry_valid &= _require_string(
            service, "digest", label=label, diagnostics=diagnostics, pattern=DIGEST
        )
        name = service.get("name")
        if isinstance(name, str):
            if name not in expected_names:
                diagnostics.append(
                    Diagnostic(
                        "invalid-schema",
                        f"{label}.name {name!r} is not an APXM service owner artifact",
                        "publish exactly apxm-compilation-service and apxm-runtime-service",
                    )
                )
            elif name in seen_names:
                diagnostics.append(
                    Diagnostic(
                        "invalid-schema",
                        f"release manifest contains duplicate service {name!r}",
                        "publish one immutable entry for each APXM service",
                    )
                )
            seen_names.add(name)
        if not entry_valid or not isinstance(name, str) or name not in expected_names:
            continue
        declared_path = _resolve_regular_file(root, root / str(service["path"]))
        selected = artifacts.get(name)
        external = _external_service_binding(name)
        selected_is_external = (
            selected is not None
            and external is not None
            and selected.resolve() == external
        )
        digest_matches_selected = (
            selected is not None
            and _digest_file(selected) == str(service.get("digest", ""))
        )
        if declared_path is None and not (selected_is_external and digest_matches_selected):
            diagnostics.append(
                Diagnostic(
                    "missing-publishable-service-artifact",
                    f"release manifest service {name!r} does not name a regular file inside the owner checkout: {service['path']}",
                    "publish the exact APXM service executable at the manifest path, or bind an immutable qualified service coordinate for a cross-platform release",
                )
            )
        elif selected is not None and declared_path != selected and not (selected_is_external and digest_matches_selected):
            diagnostics.append(
                Diagnostic(
                    "service-artifact-path-mismatch",
                    f"release manifest service {name!r} resolves to {declared_path}, but qualification selected {selected}",
                    "qualify the exact service file named by the release manifest",
                )
            )
        if name in artifact_digests and service.get("digest") != artifact_digests[name]:
            diagnostics.append(
                Diagnostic(
                    "service-artifact-digest-mismatch",
                    f"release manifest digest for {name!r} does not match the selected service bytes",
                    "publish the exact service bytes named by the manifest or regenerate the manifest",
                )
            )
    missing_names = sorted(expected_names - seen_names)
    if missing_names:
        diagnostics.append(
            Diagnostic(
                "invalid-schema",
                f"release manifest is missing service artifact(s): {', '.join(missing_names)}",
                "publish exactly one immutable entry for each APXM service",
            )
        )


def _sidecar_digest(path: Path) -> str | None:
    try:
        line = path.read_text(encoding="utf-8").splitlines()[0].strip()
    except (OSError, UnicodeDecodeError, IndexError):
        return None
    token = line.split()[0] if line else ""
    if DIGEST.fullmatch(token):
        return token
    if re.fullmatch(r"[0-9a-f]{64}", token):
        return f"sha256:{token}"
    return None


def _classify_gate_failure(returncode: int, output: str) -> str:
    """Separate infrastructure/toolchain failures from owner code failures."""

    lowered = output.casefold()
    abi_markers = (
        "e0514",
        ".rmeta",
        "rustc metadata",
        "metadata version",
        "wrong architecture",
        "undefined symbol",
        "symbol not found",
        "incompatible architecture",
        "bad cpu type",
        "elfclass",
    )
    if any(marker in lowered for marker in abi_markers):
        return "abi"
    resource_markers = (
        "no space left on device",
        "out of memory",
        "cannot allocate memory",
        "resource temporarily unavailable",
        "killed",
        "sigkill",
        "sigbus",
    )
    if returncode in {137, 143} or any(marker in lowered for marker in resource_markers):
        return "resource"
    return "code"


def _find_artifacts(
    root: Path,
    explicit: Mapping[str, str | None],
    manifest: Mapping[str, Any] | None,
) -> dict[str, Path]:
    found: dict[str, Path] = {}
    manifest_services = manifest.get("services") if manifest else None
    manifest_paths = {
        item.get("name"): item.get("path")
        for item in manifest_services
        if isinstance(item, dict) and isinstance(item.get("name"), str)
    } if isinstance(manifest_services, list) else {}
    for name, binary in SERVICE_ARTIFACTS:
        candidates: list[Path] = []
        external = _external_service_binding(name)
        if external is not None:
            # A release may be qualified on Linux and consumed from a
            # macOS/Windows owner checkout. The immutable coordinate carries
            # the exact bytes; manifest validation still binds its digest.
            candidates.append(external)
        if explicit.get(name):
            candidates.append(_rooted_path(root, explicit[name] or ""))
        env_name = (
            "APXM_COMPILATION_SERVICE_BINARY"
            if name == "compilation-service"
            else "APXM_RUNTIME_SERVICE_BINARY"
        )
        env_value = os.environ.get(env_name, "").strip()
        if env_value:
            candidates.append(_rooted_path(root, env_value))
        manifest_path = manifest_paths.get(name)
        if isinstance(manifest_path, str):
            candidates.append(root / manifest_path)
        candidates.append(root / "target" / "release" / binary)
        candidates.append(root / "deploy" / "services" / binary)
        for candidate in candidates:
            resolved = _resolve_regular_file(root, candidate)
            if resolved is None and external is not None and candidate == external:
                resolved = external
            if resolved is not None:
                found[name] = resolved
                break
    return found


def _run_gates(root: Path, gates: Sequence[str], result: Qualification) -> None:
    dekk = os.environ.get("DEKK", "").strip() or shutil.which("dekk")
    if not dekk:
        result.diagnostics.append(
            Diagnostic(
                "dekk-unavailable",
                "the `dekk` executable is not available, so owner protocol gates could not run",
                "enter the APXM Dekk environment and rerun `dekk agents release-qualification`",
            )
        )
        return
    for gate in gates:
        completed = subprocess.run(
            [dekk, "agents", gate],
            cwd=root,
            check=False,
            capture_output=True,
            text=True,
        )
        if completed.stdout:
            print(completed.stdout, end="")
        if completed.stderr:
            print(completed.stderr, end="", file=sys.stderr)
        record = {"command": f"dekk agents {gate}", "returncode": completed.returncode}
        result.gates.append(record)
        if completed.returncode != 0:
            classification = _classify_gate_failure(
                completed.returncode, f"{completed.stdout}\n{completed.stderr}"
            )
            record["classification"] = classification
            diagnostic_code = {
                "resource": "owner-gate-resource-failure",
                "abi": "owner-gate-abi-failure",
                "code": "owner-gate-code-failure",
            }[classification]
            result.diagnostics.append(
                Diagnostic(
                    diagnostic_code,
                    f"`dekk agents {gate}` exited with status {completed.returncode} ({classification} failure)",
                    {
                        "resource": "fix runner capacity, disk, process limits, or cache corruption, then rerun the gate",
                        "abi": "align the pinned compiler/toolchain and native ABI, clear only the affected cache, then rerun the gate",
                        "code": "fix the APXM owner implementation or test failure, then rerun the complete release qualification",
                    }[classification],
                )
            )


def qualify(
    root: Path,
    *,
    compilation_service_path: str | None = None,
    runtime_service_path: str | None = None,
    run_gates: bool = True,
    gates: Sequence[str] = OWNER_GATES,
) -> Qualification:
    result = Qualification()
    root = root.resolve()
    if run_gates:
        _run_gates(root, gates, result)

    if not _is_clean(root):
        result.diagnostics.append(
            Diagnostic(
                "dirty-checkout",
                "the owner checkout contains uncommitted changes; it cannot identify an immutable release",
                "publish from a clean checkout at the declared source revision; existing local edits were not changed",
            )
        )

    source_path = root / SOURCE_DESCRIPTOR_REL
    owner_path = root / OWNER_DESCRIPTOR_REL
    sidecar_path = root / OWNER_DESCRIPTOR_SIDECAR_REL
    manifest_path = root / RELEASE_MANIFEST_REL
    source = _load_json(root, source_path, result.diagnostics, "source descriptor")
    owner = _load_json(root, owner_path, result.diagnostics, "owner descriptor")
    manifest = _load_json(root, manifest_path, result.diagnostics, "release manifest")
    source_revision = _validate_source_descriptor(root, source, result.diagnostics)
    source_descriptor_file = _resolve_regular_file(root, source_path)
    source_descriptor_digest = (
        _digest_file(source_descriptor_file) if source_descriptor_file is not None else None
    )
    _validate_owner_descriptor(
        owner,
        source_revision=source_revision,
        source_descriptor_digest=source_descriptor_digest,
        diagnostics=result.diagnostics,
    )
    owner_descriptor_file = _resolve_regular_file(root, owner_path)
    owner_descriptor_digest = (
        _digest_file(owner_descriptor_file) if owner_descriptor_file is not None else None
    )

    sidecar_file = _resolve_regular_file(root, sidecar_path)
    if sidecar_file is None:
        result.diagnostics.append(
            Diagnostic(
                "missing-sidecar" if not sidecar_path.exists() else "non-regular-sidecar",
                f"owner descriptor digest sidecar is not a regular file inside the owner checkout: {OWNER_DESCRIPTOR_SIDECAR_REL}",
                "generate and publish the sidecar next to the exact owner descriptor bytes; symlinks and external files are rejected",
            )
        )
    elif owner_descriptor_digest is not None and _sidecar_digest(sidecar_file) != owner_descriptor_digest:
        result.diagnostics.append(
            Diagnostic(
                "descriptor-digest-mismatch",
                "owner descriptor sidecar does not match the exact descriptor bytes",
                "regenerate the sidecar; never hand-edit an immutable digest",
            )
        )

    artifacts = _find_artifacts(
        root,
        {
            "compilation-service": compilation_service_path,
            "runtime-service": runtime_service_path,
        },
        manifest,
    )
    result.artifacts = artifacts
    for name, binary in SERVICE_ARTIFACTS:
        artifact = artifacts.get(name)
        if artifact is None:
            result.diagnostics.append(
                Diagnostic(
                    "missing-publishable-service-artifact",
                    f"no publishable {name} executable ({binary}) is present",
                    f"build and publish the exact `{binary}` service from this source cohort, then set the matching APXM service binary variable or pass its explicit path",
                )
            )
            continue
        result.artifact_digests[name] = _digest_file(artifact)
        if not os.access(artifact, os.X_OK):
            result.diagnostics.append(
                Diagnostic(
                    "service-artifact-not-executable",
                    f"publishable {name} is not executable: {artifact}",
                    f"publish the executable `{binary}` service with execute permission",
                )
            )
    if artifacts and len(artifacts) == len(SERVICE_ARTIFACTS) and manifest is None:
        result.diagnostics.append(
            Diagnostic(
                "unattested-service-artifacts",
                "local APXM service executables were found, but they are not a published immutable service release",
                "publish both exact service binaries with the source/owner descriptors and release manifest; local builds alone cannot activate runtime",
            )
        )
    _validate_release_manifest(
        root,
        manifest,
        source_revision=source_revision,
        owner_descriptor_digest=owner_descriptor_digest,
        artifacts=artifacts,
        artifact_digests=result.artifact_digests,
        diagnostics=result.diagnostics,
    )
    if not result.ok:
        result.diagnostics.append(
            Diagnostic(
                "runtime-activation-blocked",
                "this checkout is not qualified for runtime activation",
                "publish the immutable descriptor cohort and matching Compilation/Runtime service executables; no runtime identity was inferred",
            )
        )
    return result


def _git_relative(root: Path, path: Path) -> str:
    try:
        return path.resolve().relative_to(root.resolve()).as_posix()
    except ValueError:
        return path.name


def generate_descriptors(
    root: Path,
    *,
    compilation_service_path: str,
    runtime_service_path: str,
    output_dir: Path,
    source_revision: str | None = None,
) -> tuple[Path, Path, Path, Path]:
    root = root.resolve()
    if not _is_clean(root):
        raise ValueError(
            "cannot generate immutable release descriptors from a dirty owner checkout; existing local edits were not changed"
        )
    services: dict[str, Path] = {}
    for name, binary in SERVICE_ARTIFACTS:
        raw_path = (
            compilation_service_path
            if name == "compilation-service"
            else runtime_service_path
        )
        service = _resolve_regular_file(root, _rooted_path(root, raw_path))
        if service is None:
            raise ValueError(
                f"publishable {name} must be a regular file inside the owner checkout: {raw_path}; supply the real {binary} binary"
            )
        if not os.access(service, os.X_OK):
            raise ValueError(f"publishable {name} is not executable: {service}")
        services[name] = service
    revision = source_revision or _git_revision(root)
    if revision is None or not HEX40.fullmatch(revision):
        raise ValueError("cannot generate descriptors without a full lowercase Git source revision")
    if _git_text(root, "rev-parse", f"{revision}^{{commit}}") != revision:
        raise ValueError(f"source revision is not present in the owner checkout: {revision}")
    if not _git_is_ancestor(root, revision, "HEAD"):
        raise ValueError(f"source revision is not an ancestor of the owner checkout HEAD: {revision}")

    source = {
        "schema_version": SOURCE_DESCRIPTOR_SCHEMA,
        "semantic_owner": "agents",
        "selection": "immutable-source-cohort",
        "source_revision": revision,
    }
    source_bytes = _canonical_json(source)
    source_digest = _digest_bytes(source_bytes)
    owner = {
        "schema_version": OWNER_DESCRIPTOR_SCHEMA,
        "semantic_owner": "agents",
        "source_revision": revision,
        "source_descriptor_digest": source_digest,
        "artifact_kind": ARTIFACT_KIND,
        "compilation_protocol": COMPILATION_PROTOCOL_VERSION,
        "runtime_protocol": RUNTIME_PROTOCOL_VERSION,
    }
    owner_bytes = _canonical_json(owner)
    owner_digest = _digest_bytes(owner_bytes)
    manifest = {
        "schema_version": RELEASE_MANIFEST_SCHEMA,
        "semantic_owner": "agents",
        "source_revision": revision,
        "artifact_kind": ARTIFACT_KIND,
        "services": [
            {
                "name": name,
                "path": _git_relative(root, services[name]),
                "digest": _digest_file(services[name]),
            }
            for name, _ in SERVICE_ARTIFACTS
        ],
        "owner_descriptor_digest": owner_digest,
        "integrity_algorithm": "sha256",
    }
    manifest_bytes = _canonical_json(manifest)

    source_out = output_dir / SOURCE_DESCRIPTOR_REL
    owner_out = output_dir / OWNER_DESCRIPTOR_REL
    sidecar_out = output_dir / OWNER_DESCRIPTOR_SIDECAR_REL
    manifest_out = output_dir / RELEASE_MANIFEST_REL
    outputs = (
        (source_out, source_bytes),
        (owner_out, owner_bytes),
        (sidecar_out, f"{owner_digest}  {OWNER_DESCRIPTOR_REL.as_posix()}\n".encode("utf-8")),
        (manifest_out, manifest_bytes),
    )
    for path, payload in outputs:
        if path.is_symlink() or (path.exists() and (not path.is_file() or path.read_bytes() != payload)):
            raise ValueError(
                f"refusing to overwrite existing release input with different bytes: {path}"
            )
    for path, payload in outputs:
        path.parent.mkdir(parents=True, exist_ok=True)
        if not path.exists():
            path.write_bytes(payload)
    return source_out, owner_out, sidecar_out, manifest_out


def _print_result(result: Qualification, *, as_json: bool, root: Path = REPOSITORY_ROOT) -> None:
    root = root.resolve()
    source = _load_json(root, SOURCE_DESCRIPTOR_REL, [], "source descriptor") or {}
    owner = _load_json(root, OWNER_DESCRIPTOR_REL, [], "owner descriptor") or {}
    manifest = _load_json(root, RELEASE_MANIFEST_REL, [], "release manifest") or {}
    source_file = _resolve_regular_file(root, root / SOURCE_DESCRIPTOR_REL)
    owner_file = _resolve_regular_file(root, root / OWNER_DESCRIPTOR_REL)
    manifest_file = _resolve_regular_file(root, root / RELEASE_MANIFEST_REL)
    payload = {
        "schema": "apxm.agents.release-qualification.v1",
        "owner": "agents",
        "evidence_root": str(root),
        "qualified": result.ok,
        "source_revision": source.get("source_revision"),
        "source_descriptor_digest": _digest_file(source_file) if source_file else None,
        "owner_descriptor_digest": _digest_file(owner_file) if owner_file else None,
        "release_manifest_digest": _digest_file(manifest_file) if manifest_file else None,
        "manifest_services": manifest.get("services"),
        "services": {name: str(path) for name, path in result.artifacts.items()},
        "service_digests": result.artifact_digests,
        "gates": result.gates,
        "diagnostics": [
            {"code": item.code, "message": item.message, "remediation": item.remediation}
            for item in result.diagnostics
        ],
    }
    if as_json:
        print(json.dumps(payload, indent=2, sort_keys=True))
    else:
        print("APXM owner release qualification: PASS" if result.ok else "APXM owner release qualification: FAIL")
        for gate in result.gates:
            print(f"gate: {gate['command']} -> {gate['returncode']}")
        for diagnostic in result.diagnostics:
            print(diagnostic.render(), file=sys.stderr)


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="release_qualification.py")
    subparsers = parser.add_subparsers(dest="mode")
    qualify_parser = subparsers.add_parser("qualify", help="run owner gates and qualify a release cohort")
    qualify_parser.add_argument("--root", type=Path, default=REPOSITORY_ROOT)
    qualify_parser.add_argument("--compilation-service", help="path to the published apxm-compilation-service executable")
    qualify_parser.add_argument("--runtime-service", help="path to the published apxm-runtime-service executable")
    qualify_parser.add_argument("--skip-gates", action="store_true", help="skip only for offline metadata tests")
    qualify_parser.add_argument("--json", action="store_true", dest="as_json")
    generate_parser = subparsers.add_parser("generate", help="generate descriptors from a real artifact")
    generate_parser.add_argument("--root", type=Path, default=REPOSITORY_ROOT)
    generate_parser.add_argument("--compilation-service", required=True)
    generate_parser.add_argument("--runtime-service", required=True)
    generate_parser.add_argument("--output-dir", type=Path, required=True)
    generate_parser.add_argument("--source-revision")
    args = parser.parse_args(list(argv) if argv is not None else None)
    mode = args.mode or "qualify"
    if mode == "generate":
        try:
            outputs = generate_descriptors(
                args.root,
                compilation_service_path=args.compilation_service,
                runtime_service_path=args.runtime_service,
                output_dir=args.output_dir,
                source_revision=args.source_revision,
            )
        except (OSError, ValueError) as exc:
            print(f"descriptor generation failed: {exc}", file=sys.stderr)
            return 2
        for path in outputs:
            print(path)
        return 0
    result = qualify(
        args.root,
        compilation_service_path=args.compilation_service,
        runtime_service_path=args.runtime_service,
        run_gates=not args.skip_gates,
    )
    _print_result(result, as_json=args.as_json, root=args.root)
    return 0 if result.ok else 1


if __name__ == "__main__":
    raise SystemExit(main())
