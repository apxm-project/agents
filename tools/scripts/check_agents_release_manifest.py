#!/usr/bin/env python3
"""Build and validate the product-neutral Agents owner release cohort."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import subprocess
from pathlib import Path
from typing import Any


REPO_ROOT = Path(__file__).resolve().parents[2]
MANIFEST_PATH = REPO_ROOT / "release" / "manifests" / "agents-release.v1.json"
EVIDENCE_PATH = REPO_ROOT / "evidence" / "release" / "agents-release-evidence.v1.json"
DESCRIPTOR_PATH = "contracts/descriptors/apxm.agents-owner-descriptor.v1.json"
DESCRIPTOR_SIDECAR_PATH = "contracts/descriptors/apxm.agents-owner-descriptor.v1.sha256"
OWNER_REPOSITORY = "https://github.com/apxm-project/agents.git"
REFERENCE_HOST_RELEASE_PATH = (
    "contracts/reference-host/manifests/apxm.reference-host-release-manifest.v1.json"
)
REFERENCE_HOST_EXECUTION_PATH = (
    "contracts/reference-host/manifests/apxm.reference-host-execution-manifest.v1.json"
)
REFERENCE_HOST_HARNESS_PATH = "crates/tools/cli/tests/reference_host_jsonl.rs"
REFERENCE_HOST_IMAGE_MANIFEST_PATH = "deploy/reference-host/image-manifest.v1.json"
REFERENCE_HOST_IMAGE_SIDECAR_PATH = "deploy/reference-host/image-manifest.v1.sha256"
EXACT_DIGEST = re.compile(r"sha256:[0-9a-f]{64}\Z")
EXACT_REVISION = re.compile(r"[0-9a-f]{40}\Z")
FORBIDDEN_RELEASE_TERMS = ("signature", "trust root", "pki")


class ReleaseManifestError(ValueError):
    """Raised when the owner release evidence is not exact."""


def digest_bytes(value: bytes) -> str:
    return "sha256:" + hashlib.sha256(value).hexdigest()


def json_bytes(value: dict[str, Any]) -> bytes:
    return (json.dumps(value, indent=2, sort_keys=True) + "\n").encode("utf-8")


def canonical(value: dict[str, Any]) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":")).encode("utf-8")


def load_json_bytes(value: bytes, label: str) -> dict[str, Any]:
    try:
        decoded = json.loads(value)
    except json.JSONDecodeError as error:
        raise ReleaseManifestError(f"{label} is not valid JSON: {error}") from error
    if not isinstance(decoded, dict):
        raise ReleaseManifestError(f"{label} must be a JSON object")
    return decoded


def git(repo_root: Path, *args: str) -> bytes:
    environment = os.environ.copy()
    environment.pop("DYLD_LIBRARY_PATH", None)
    environment.pop("DYLD_FALLBACK_LIBRARY_PATH", None)
    result = subprocess.run(
        ["git", "-C", str(repo_root), *args],
        capture_output=True,
        check=False,
        env=environment,
    )
    if result.returncode:
        detail = result.stderr.decode("utf-8", errors="replace").strip()
        raise ReleaseManifestError(f"git {' '.join(args)} failed: {detail}")
    return result.stdout


def git_text(repo_root: Path, *args: str) -> str:
    return git(repo_root, *args).decode("utf-8").strip()


def git_file(repo_root: Path, revision: str, path: str) -> bytes:
    return git(repo_root, "show", f"{revision}:{path}")


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ReleaseManifestError(message)


def descriptor_from_revision(revision: str) -> tuple[dict[str, Any], bytes]:
    raw = git_file(REPO_ROOT, revision, DESCRIPTOR_PATH)
    descriptor = load_json_bytes(raw, "owner descriptor")
    require(descriptor.get("semantic_owner") == "agents", "owner descriptor semantic_owner is not agents")
    sidecar = git_file(REPO_ROOT, revision, DESCRIPTOR_SIDECAR_PATH).decode("utf-8").split()
    require(sidecar and sidecar[0] == digest_bytes(raw), "owner descriptor sidecar is stale")
    return descriptor, raw


def source_facts(revision: str) -> dict[str, str]:
    require(
        EXACT_REVISION.fullmatch(revision) is not None,
        "source revision must be a full lowercase Git revision",
    )
    require(
        git_text(REPO_ROOT, "rev-parse", f"{revision}^{{commit}}") == revision,
        "source revision is not committed",
    )
    current = git_text(REPO_ROOT, "rev-parse", "HEAD")
    require(
        git(REPO_ROOT, "merge-base", "--is-ancestor", revision, current) == b"",
        "source revision is not an ancestor of the current checkout",
    )
    tree = git_text(REPO_ROOT, "rev-parse", f"{revision}^{{tree}}")
    return {
        "repository": OWNER_REPOSITORY,
        "revision": revision,
        "tree_digest": digest_bytes(git(REPO_ROOT, "cat-file", "tree", tree)),
        "archive_digest": digest_bytes(git(REPO_ROOT, "archive", "--format=tar", revision)),
    }


def descriptor_contracts(descriptor: dict[str, Any]) -> dict[str, Any]:
    return {
        "constitution": descriptor["constitution"],
        "owned_port_contracts": [
            {"digest": item["digest"], "port_contract_id": item["port_contract_id"]}
            for item in descriptor["owned_port_contracts"]
        ],
        "owned_schemas": [
            {"digest": item["digest"], "schema_id": item["schema_id"]}
            for item in descriptor["owned_schemas"]
        ],
        "referenced_common_envelopes": descriptor["referenced_common_envelopes"],
    }


def reference_host_cohort(revision: str) -> dict[str, Any]:
    release_path = REFERENCE_HOST_RELEASE_PATH
    execution_path = REFERENCE_HOST_EXECUTION_PATH
    release = load_json_bytes(git_file(REPO_ROOT, revision, release_path), "reference-host release manifest")
    execution = load_json_bytes(git_file(REPO_ROOT, revision, execution_path), "reference-host execution manifest")
    profile_cohort = release.get("profile_cohort")
    require(profile_cohort == ["embedded", "reference-host"], "reference-host profile cohort drifted")

    publication_cohort: dict[str, list[dict[str, Any]]] = {}
    published = release.get("publication_cohort")
    require(isinstance(published, dict), "reference-host publication cohort is missing")
    for group, identifier_field in (
        ("manifests", "schema_version"),
        ("schemas", "schema_id"),
        ("vectors", "schema_id"),
    ):
        entries = published.get(group)
        require(isinstance(entries, list), f"reference-host publication cohort {group} is missing")
        normalized: list[dict[str, Any]] = []
        for entry in entries:
            require(isinstance(entry, dict), f"reference-host {group} entry is not an object")
            identifier = entry.get(identifier_field)
            path = entry.get("path")
            require(
                isinstance(identifier, str) and isinstance(path, str),
                f"reference-host {group} entry is incomplete",
            )
            execution_entry = next(
                (
                    item
                    for item in execution.values()
                    if isinstance(item, dict)
                    and item.get(identifier_field) == identifier
                    and item.get("path") == path
                ),
                None,
            )
            require(
                execution_entry is not None or group == "manifests",
                f"reference-host execution manifest lacks {identifier}",
            )
            normalized.append(
                {
                    identifier_field: identifier,
                    "path": f"contracts/{path}",
                    "exact_bytes_digest": digest_bytes(
                        git_file(REPO_ROOT, revision, f"contracts/{path}")
                    ),
                }
            )
        publication_cohort[group] = normalized

    golden_vectors: list[dict[str, Any]] = []
    for entry in release.get("golden_vectors", []):
        require(isinstance(entry, dict), "reference-host golden vector entry is not an object")
        path = entry.get("path")
        require(isinstance(path, str), "reference-host golden vector path is missing")
        golden_vectors.append(
            {
                "vector_id": entry["vector_id"],
                "path": f"contracts/{path}",
                "exact_bytes_digest": digest_bytes(
                    git_file(REPO_ROOT, revision, f"contracts/{path}")
                ),
                "profiles": entry["profiles"],
            }
        )

    return {
        "profile_cohort": profile_cohort,
        "release_manifest": {
            "path": release_path,
            "exact_bytes_digest": digest_bytes(git_file(REPO_ROOT, revision, release_path)),
        },
        "execution_manifest": {
            "path": execution_path,
            "exact_bytes_digest": digest_bytes(git_file(REPO_ROOT, revision, execution_path)),
        },
        "publication_cohort": publication_cohort,
        "golden_vectors": golden_vectors,
        "harness": {
            "path": REFERENCE_HOST_HARNESS_PATH,
            "exact_bytes_digest": digest_bytes(
                git_file(REPO_ROOT, revision, REFERENCE_HOST_HARNESS_PATH)
            ),
        },
        "constraints": {
            "http_surface": release["constraints"]["http_surface"],
            "apxm_server_dependency": release["constraints"]["apxm_server_dependency"],
            "source_checkout_fallback": release["constraints"]["source_checkout_fallback"],
            "schema_aliases": release["constraints"]["schema_aliases"],
            "mixed_generation": release["constraints"]["mixed_generation"],
            "downstream_dependency": "absent",
        },
        "evidence_scope": "committed manifests, schemas, vectors, and harness bytes only",
    }


def reference_host_image(
    source_revision: str,
    artifact_revision: str,
    image_id: str,
    binary_digest: str,
) -> dict[str, Any]:
    require(
        EXACT_REVISION.fullmatch(artifact_revision) is not None,
        "image artifact revision must be a full lowercase Git revision",
    )
    require(
        git_text(REPO_ROOT, "rev-parse", f"{artifact_revision}^{{commit}}")
        == artifact_revision,
        "image artifact revision is not committed",
    )
    require(
        git(
            REPO_ROOT,
            "merge-base",
            "--is-ancestor",
            source_revision,
            artifact_revision,
        )
        == b"",
        "image artifact revision is not a descendant of the source revision",
    )
    require(
        EXACT_DIGEST.fullmatch(image_id) is not None,
        "image id must be an exact sha256 digest",
    )
    require(
        EXACT_DIGEST.fullmatch(binary_digest) is not None,
        "reference-host binary digest must be an exact sha256 digest",
    )
    raw = git_file(REPO_ROOT, artifact_revision, REFERENCE_HOST_IMAGE_MANIFEST_PATH)
    image_manifest = load_json_bytes(raw, "reference-host image manifest")
    sidecar = git_file(
        REPO_ROOT, artifact_revision, REFERENCE_HOST_IMAGE_SIDECAR_PATH
    ).decode("utf-8").splitlines()
    require(len(sidecar) == 2, "reference-host image sidecar shape drifted")
    require(
        sidecar[0]
        == f"{digest_bytes(raw)}  {REFERENCE_HOST_IMAGE_MANIFEST_PATH}",
        "reference-host image sidecar is stale",
    )
    require(
        sidecar[1] == "digest_scope: exact repository bytes",
        "reference-host image sidecar scope drifted",
    )
    require(
        image_manifest.get("semantic_owner") == "agents",
        "reference-host image owner drifted",
    )
    require(
        image_manifest.get("source_revision") == source_revision,
        "reference-host image source revision drifted",
    )
    require(
        image_manifest.get("reviewed_carrier_revision")
        == "8f767541413d0905d638b288a797c5825ceb1c76",
        "reference-host image reviewed carrier drifted",
    )
    semantic_payload = dict(image_manifest)
    declared_semantic_digest = semantic_payload.pop("manifest_digest", None)
    require(
        declared_semantic_digest == digest_bytes(canonical(semantic_payload)),
        "reference-host image semantic digest is stale",
    )
    release_manifest = image_manifest.get("reference_host_release_manifest")
    require(
        isinstance(release_manifest, dict)
        and release_manifest.get("path") == REFERENCE_HOST_RELEASE_PATH
        and release_manifest.get("digest")
        == digest_bytes(
            git_file(REPO_ROOT, source_revision, REFERENCE_HOST_RELEASE_PATH)
        ),
        "reference-host image release-manifest binding drifted",
    )
    for field in ("build_recipe", "platform", "lifecycle_gate", "runtime"):
        require(
            isinstance(image_manifest.get(field), dict),
            f"reference-host image manifest lacks {field}",
        )
    build_recipe = image_manifest["build_recipe"]
    for path_field, digest_field, revision in (
        ("dockerfile_path", "dockerfile_digest", artifact_revision),
        ("dockerignore_path", "dockerignore_digest", artifact_revision),
        ("cargo_lock_path", "cargo_lock_digest", source_revision),
    ):
        path = build_recipe.get(path_field)
        require(isinstance(path, str), f"reference-host image lacks {path_field}")
        require(
            build_recipe.get(digest_field)
            == digest_bytes(git_file(REPO_ROOT, revision, path)),
            f"reference-host image {digest_field} drifted",
        )
    require(
        image_manifest["platform"]
        == {
            "os": "linux",
            "architecture": "arm64",
            "elf_machine": "AArch64",
            "rust_target": "aarch64-unknown-linux-gnu",
        },
        "reference-host image platform drifted",
    )
    lifecycle_gate = image_manifest["lifecycle_gate"]
    for path_field, digest_field in (
        ("harness_path", "harness_digest"),
        ("vector_path", "vector_digest"),
    ):
        path = lifecycle_gate.get(path_field)
        require(isinstance(path, str), f"reference-host image lacks {path_field}")
        require(
            lifecycle_gate.get(digest_field)
            == digest_bytes(git_file(REPO_ROOT, source_revision, path)),
            f"reference-host image {digest_field} drifted",
        )
    require(
        image_manifest["runtime"]
        == {
            "entrypoint": ["/usr/local/bin/apxm-reference-host"],
            "user": "65532:65532",
            "source_mount": "absent",
        },
        "reference-host image runtime drifted",
    )
    return {
        "artifact_revision": artifact_revision,
        "manifest": {
            "path": REFERENCE_HOST_IMAGE_MANIFEST_PATH,
            "exact_bytes_digest": digest_bytes(raw),
            "semantic_digest": image_manifest["manifest_digest"],
        },
        "sidecar": {
            "path": REFERENCE_HOST_IMAGE_SIDECAR_PATH,
            "exact_bytes_digest": digest_bytes(
                git_file(
                    REPO_ROOT,
                    artifact_revision,
                    REFERENCE_HOST_IMAGE_SIDECAR_PATH,
                )
            ),
        },
        "build_recipe": image_manifest["build_recipe"],
        "platform": image_manifest["platform"],
        "lifecycle_gate": image_manifest["lifecycle_gate"],
        "runtime": image_manifest["runtime"],
        "built_artifact": {
            "image_id": image_id,
            "binary_exact_bytes_digest": binary_digest,
            "elf_type": "ELF64 PIE",
            "elf_machine": "AArch64",
        },
    }


def build_manifest(
    revision: str,
    artifact_revision: str,
    image_id: str,
    binary_digest: str,
) -> dict[str, Any]:
    descriptor, descriptor_bytes = descriptor_from_revision(revision)
    source = source_facts(revision)
    image = reference_host_image(
        revision, artifact_revision, image_id, binary_digest
    )
    return {
        "artifacts": [
            {
                "kind": "source-archive",
                "digest": source["archive_digest"],
                "build_decision": "built",
                "provenance": "git archive of the pinned owner revision",
            },
            {
                "kind": "oci-image",
                "digest": image_id,
                "build_decision": "built",
                "platform": "linux/arm64",
                "provenance": "pinned Agents-owned Docker recipe and extracted executable digest",
            },
        ],
        "contracts": descriptor_contracts(descriptor),
        "owner_descriptor": {
            "path": DESCRIPTOR_PATH,
            "exact_bytes_digest": digest_bytes(descriptor_bytes),
        },
        "provenance": {
            "source_of_truth": "committed owner descriptor, image recipe, and git object bytes",
            "serving_time_dependency_discovery": False,
            "source_mounts_in_reference_profile": False,
        },
        "reference_host_image": image,
        "reference_host_release": reference_host_cohort(revision),
        "release_id": f"apxm-agents-{revision[:12]}-arm64-{artifact_revision[:12]}",
        "release_kind": "source-and-linux-arm64-image-provenance",
        "schema_version": "apxm.owner-release-manifest.v1",
        "semantic_owner": "agents",
        "source": source,
    }


def build_evidence(manifest: dict[str, Any], manifest_digest: str) -> dict[str, Any]:
    return {
        "schema_version": "apxm.agents-release-evidence.v1",
        "release_kind": manifest["release_kind"],
        "semantic_owner": manifest["semantic_owner"],
        "release_id": manifest["release_id"],
        "release_manifest": {
            "path": "release/manifests/agents-release.v1.json",
            "exact_bytes_digest": manifest_digest,
        },
        "source": manifest["source"],
        "owner_descriptor": manifest["owner_descriptor"],
        "reference_host_image": manifest["reference_host_image"],
        "reference_host_release": manifest["reference_host_release"],
        "provenance": manifest["provenance"],
        "verification": {
            "gate": "tools/scripts/check_agents_release_manifest.py",
            "scope": "digest-bound committed owner, image recipe, extracted ELF, and lifecycle evidence",
            "runtime_execution": "linux_arm64_lifecycle_vectors_passed",
        },
    }


def read_working_json(path: Path, label: str) -> dict[str, Any]:
    try:
        return load_json_bytes(path.read_bytes(), label)
    except OSError as error:
        raise ReleaseManifestError(f"cannot read {label}: {error}") from error


def validate_manifest_payload(
    manifest: dict[str, Any], *, manifest_path: Path = MANIFEST_PATH
) -> dict[str, Any]:
    expected = build_manifest(
        manifest["source"]["revision"],
        manifest["reference_host_image"]["artifact_revision"],
        manifest["reference_host_image"]["built_artifact"]["image_id"],
        manifest["reference_host_image"]["built_artifact"][
            "binary_exact_bytes_digest"
        ],
    )
    require(
        manifest == expected,
        "owner release manifest differs from committed source and cohort bytes",
    )
    encoded = json.dumps(manifest, sort_keys=True).lower()
    for term in FORBIDDEN_RELEASE_TERMS:
        require(term not in encoded, f"owner release manifest contains forbidden term: {term}")
    require(manifest_path.is_file(), "owner release manifest is missing")
    manifest_digest = digest_bytes(manifest_path.read_bytes())
    evidence = read_working_json(EVIDENCE_PATH, "owner release evidence")
    expected_evidence = build_evidence(manifest, manifest_digest)
    require(
        evidence == expected_evidence,
        "owner release evidence differs from the owner manifest",
    )
    require(
        digest_bytes(json_bytes(manifest)) == manifest_digest,
        "owner release manifest bytes are not canonical",
    )
    return {
        "status": "pass",
        "release_id": manifest["release_id"],
        "revision": manifest["source"]["revision"],
        "tree_digest": manifest["source"]["tree_digest"],
        "archive_digest": manifest["source"]["archive_digest"],
        "manifest_digest": manifest_digest,
        "evidence_digest": digest_bytes(EVIDENCE_PATH.read_bytes()),
    }


def validate_manifest(
    *, require_clean: bool = True, manifest_path: Path = MANIFEST_PATH
) -> dict[str, Any]:
    if require_clean:
        status = git_text(
            REPO_ROOT, "status", "--porcelain=v1", "--untracked-files=all"
        )
        require(not status, "owner checkout is dirty; release validation is fail-closed")
    manifest = read_working_json(manifest_path, "owner release manifest")
    return validate_manifest_payload(manifest, manifest_path=manifest_path)


def write_json(path: Path, value: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(json_bytes(value))


def write_release_evidence(
    revision: str,
    artifact_revision: str,
    image_id: str,
    binary_digest: str,
) -> None:
    manifest = build_manifest(
        revision, artifact_revision, image_id, binary_digest
    )
    write_json(MANIFEST_PATH, manifest)
    write_json(EVIDENCE_PATH, build_evidence(manifest, digest_bytes(json_bytes(manifest))))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--check", action="store_true", help="validate committed release evidence"
    )
    parser.add_argument(
        "--allow-dirty",
        action="store_true",
        help="allow a dirty tree for pre-commit validation",
    )
    parser.add_argument(
        "--write",
        action="store_true",
        help="write evidence for an explicit committed revision",
    )
    parser.add_argument("--revision", help="full Git revision used by --write")
    parser.add_argument(
        "--artifact-revision", help="committed image-recipe revision used by --write"
    )
    parser.add_argument("--image-id", help="locally built Linux/arm64 image digest")
    parser.add_argument(
        "--binary-digest", help="extracted Linux/arm64 reference-host digest"
    )
    args = parser.parse_args()
    try:
        if args.write:
            require(args.revision is not None, "--revision is required with --write")
            require(
                args.artifact_revision is not None,
                "--artifact-revision is required with --write",
            )
            require(args.image_id is not None, "--image-id is required with --write")
            require(
                args.binary_digest is not None,
                "--binary-digest is required with --write",
            )
            write_release_evidence(
                args.revision,
                args.artifact_revision,
                args.image_id,
                args.binary_digest,
            )
            result = validate_manifest(require_clean=False)
        elif args.check:
            result = validate_manifest(require_clean=not args.allow_dirty)
        else:
            parser.error("one of --check or --write is required")
    except (ReleaseManifestError, KeyError) as error:
        print(f"FAIL: {error}")
        return 1
    print("PASS: Agents owner release manifest")
    for key in (
        "release_id",
        "revision",
        "tree_digest",
        "archive_digest",
        "manifest_digest",
        "evidence_digest",
    ):
        print(f"{key}: {result[key]}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
