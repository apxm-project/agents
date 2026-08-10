#!/usr/bin/env python3
"""Build and dynamically attest the canonical Linux/arm64 reference-host image."""

from __future__ import annotations

import argparse
import copy
import hashlib
import importlib.util
import json
import os
import subprocess
import tarfile
import tempfile
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[2]
IMAGE_CHECKER_PATH = ROOT / "tools/scripts/check_reference_host_image.py"
LIFECYCLE_PATH = ROOT / "tools/scripts/reference_host_lifecycle_receipt.py"
RECEIPT_PATH = ROOT / "evidence/release/reference-host-image-build-receipt.v1.json"
IMAGE_MANIFEST_PATH = ROOT / "deploy/reference-host/image-manifest.v1.json"
RELEASE_MANIFEST_PATH = (
    ROOT
    / "contracts/reference-host/manifests/apxm.reference-host-release-manifest.v1.json"
)
OWNER_DESCRIPTOR_PATH = ROOT / "contracts/descriptors/apxm.agents-owner-descriptor.v1.json"
SOURCE_REVISION = "140ce5b16b4bc4a078800b52fffbfc8004169ee1"
REVIEWED_CARRIER_REVISION = "8f767541413d0905d638b288a797c5825ceb1c76"
RECIPE_REVISION = "887692df1ba13e84e214b6d8128a8a7c2c80e836"
PORT_BINDINGS_DIGEST = "sha256:9cfeb1dee6cfb05086c170fdf9e8e8092c10f2d9410c2e48fd2377351b1a3df4"
RESOURCE_CEILING_DIGEST = "sha256:32fe0355dd58906703ee8731a407b77831954e255a40cc9df3f6e704f8579485"
GATE_FILES = (
    ".dekk.toml",
    "tools/scripts/reference_host_image_gate.py",
    "tools/scripts/check_reference_host_image.py",
    "tools/scripts/reference_host_lifecycle_receipt.py",
    "tools/tests/test_reference_host_image_gate.py",
)


class ImageGateError(RuntimeError):
    """The dynamic image observation is incomplete or inconsistent."""


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ImageGateError(message)


def digest_bytes(value: bytes) -> str:
    return "sha256:" + hashlib.sha256(value).hexdigest()


def json_bytes(value: dict[str, Any]) -> bytes:
    return (json.dumps(value, indent=2, sort_keys=True) + "\n").encode("utf-8")


def load_module(path: Path, name: str) -> Any:
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise ImageGateError(f"cannot load {path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


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
        raise ImageGateError(f"git {' '.join(args)} failed: {detail}")
    return result.stdout


def git_text(*args: str) -> str:
    return git(*args).decode("utf-8").strip()


def git_file(revision: str, path: str) -> bytes:
    return git("show", f"{revision}:{path}")


def run(command: list[str], *, input_text: str | None = None) -> subprocess.CompletedProcess[str]:
    result = subprocess.run(
        command,
        cwd=ROOT,
        input=input_text,
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode:
        detail = result.stderr.strip() or result.stdout.strip()
        raise ImageGateError(f"{' '.join(command)} failed: {detail}")
    return result


def docker_json(*args: str) -> Any:
    result = run(["docker", *args])
    try:
        return json.loads(result.stdout)
    except json.JSONDecodeError as error:
        raise ImageGateError(f"docker {' '.join(args)} returned invalid JSON") from error


def exact_gate_files(gate_revision: str) -> list[dict[str, str]]:
    require(
        git_text("rev-parse", f"{gate_revision}^{{commit}}") == gate_revision,
        "receipt gate revision is not committed",
    )
    require(
        git("merge-base", "--is-ancestor", gate_revision, "HEAD") == b"",
        "receipt gate revision is not an ancestor of HEAD",
    )
    result: list[dict[str, str]] = []
    for path in GATE_FILES:
        committed = git_file(gate_revision, path)
        require((ROOT / path).read_bytes() == committed, f"dynamic gate file drifted: {path}")
        result.append({"path": path, "digest": digest_bytes(committed)})
    return result


def materialize_build_context(destination: Path) -> None:
    archive_path = destination / "source.tar"
    archive_path.write_bytes(git("archive", "--format=tar", SOURCE_REVISION))
    context = destination / "context"
    context.mkdir()
    with tarfile.open(archive_path, "r:") as archive:
        archive.extractall(context, filter="data")
    for path in (".dockerignore", "deploy/reference-host/Dockerfile"):
        target = context / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(git_file(RECIPE_REVISION, path))


def build_image(context: Path, tag: str) -> str:
    run(
        [
            "docker",
            "build",
            "--no-cache",
            "--platform=linux/arm64",
            "--provenance=false",
            "--file",
            str(context / "deploy/reference-host/Dockerfile"),
            "--tag",
            tag,
            str(context),
        ]
    )
    inspected = docker_json("image", "inspect", tag)
    require(isinstance(inspected, list) and len(inspected) == 1, "image inspect must return one image")
    image_id = inspected[0].get("Id")
    require(isinstance(image_id, str), "image inspect omitted Id")
    return image_id


def expected_labels() -> dict[str, str]:
    image_manifest = json.loads(IMAGE_MANIFEST_PATH.read_text(encoding="utf-8"))
    return {
        "io.apxm.reference-host-release-manifest-digest": image_manifest[
            "reference_host_release_manifest"
        ]["digest"],
        "io.apxm.reviewed-carrier-revision": REVIEWED_CARRIER_REVISION,
        "org.opencontainers.image.revision": SOURCE_REVISION,
        "org.opencontainers.image.source": "https://github.com/apxm-project/agents.git",
        "org.opencontainers.image.title": "APXM reference host",
    }


def inspect_image(tag: str) -> dict[str, Any]:
    rows = docker_json("image", "inspect", tag)
    require(isinstance(rows, list) and len(rows) == 1, "image inspect must return one row")
    row = rows[0]
    config = row.get("Config")
    require(isinstance(config, dict), "image Config is missing")
    observed = {
        "image_id": row.get("Id"),
        "os": row.get("Os"),
        "architecture": row.get("Architecture"),
        "user": config.get("User"),
        "entrypoint": config.get("Entrypoint"),
        "labels": config.get("Labels"),
    }
    require(observed["os"] == "linux", "image OS is not linux")
    require(observed["architecture"] == "arm64", "image architecture is not arm64")
    require(observed["user"] == "65532:65532", "image User is not the owner runtime user")
    require(
        observed["entrypoint"] == ["/usr/local/bin/apxm-reference-host"],
        "image entrypoint drifted",
    )
    require(observed["labels"] == expected_labels(), "image labels drifted")
    return observed


def extract_binary(tag: str, destination: Path) -> dict[str, str]:
    container_id = run(["docker", "create", tag]).stdout.strip()
    require(container_id, "docker create omitted the container id")
    try:
        run(
            [
                "docker",
                "cp",
                f"{container_id}:/usr/local/bin/apxm-reference-host",
                str(destination),
            ]
        )
    finally:
        subprocess.run(["docker", "rm", container_id], cwd=ROOT, capture_output=True, check=False)
    data = destination.read_bytes()
    require(len(data) >= 20 and data[:4] == b"\x7fELF", "reference host is not ELF")
    require(data[4] == 2 and data[5] == 1, "reference host is not little-endian ELF64")
    require(int.from_bytes(data[16:18], "little") == 3, "reference host is not ELF PIE")
    require(int.from_bytes(data[18:20], "little") == 183, "reference host is not AArch64")
    return {
        "path": "/usr/local/bin/apxm-reference-host",
        "exact_bytes_digest": digest_bytes(data),
        "elf_class": "ELF64",
        "elf_type": "PIE",
        "elf_machine": "AArch64",
    }


class DockerTransport:
    """Run each JSONL lifecycle case through the inspected final image."""

    def __init__(self, image: str, runtime_root: Path) -> None:
        self.process = subprocess.Popen(
            [
                "docker",
                "run",
                "--rm",
                "--interactive",
                "--platform=linux/arm64",
                "--mount",
                f"type=bind,src={runtime_root},dst=/runtime,readonly",
                image,
                "--startup-input",
                "/runtime/startup-input.json",
            ],
            cwd=ROOT,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        require(self.process.stdin is not None and self.process.stdout is not None, "docker transport has no pipes")

    def request(self, payload: dict[str, Any]) -> dict[str, Any]:
        assert self.process.stdin is not None
        assert self.process.stdout is not None
        self.process.stdin.write(json.dumps(payload, sort_keys=True) + "\n")
        self.process.stdin.flush()
        line = self.process.stdout.readline()
        require(bool(line), "image transport closed before returning a response")
        try:
            response = json.loads(line)
        except json.JSONDecodeError as error:
            raise ImageGateError("image transport returned invalid JSON") from error
        require(isinstance(response, dict), "image transport response is not an object")
        return response

    def close(self) -> None:
        assert self.process.stdin is not None
        self.process.stdin.close()
        stderr = self.process.stderr.read().strip() if self.process.stderr is not None else ""
        exit_code = self.process.wait()
        require(exit_code == 0, f"image transport exited {exit_code}: {stderr}")


def startup_input(runtime_root: Path) -> dict[str, Any]:
    release_bytes = git_file(SOURCE_REVISION, str(RELEASE_MANIFEST_PATH.relative_to(ROOT)))
    release_target = runtime_root / "contracts/reference-host/manifests/apxm.reference-host-release-manifest.v1.json"
    release_target.parent.mkdir(parents=True)
    release_target.write_bytes(release_bytes)
    descriptor_bytes = git_file(SOURCE_REVISION, str(OWNER_DESCRIPTOR_PATH.relative_to(ROOT)))
    descriptor = json.loads(descriptor_bytes)
    payload = {
        "schema_version": "apxm.reference-host-startup-input.v1",
        "semantic_owner": "agents",
        "owner_executable": "apxm-reference-host",
        "owner_executable_path": "crates/tools/cli/src/bin/reference_host.rs",
        "transport_protocol": "jsonl-stdin-stdout",
        "reference_host_release_manifest": {
            "path": "/runtime/contracts/reference-host/manifests/apxm.reference-host-release-manifest.v1.json",
            "digest": digest_bytes(release_bytes),
        },
        "release_digest": digest_bytes(release_bytes),
        "port_bindings_digest": PORT_BINDINGS_DIGEST,
        "resource_ceiling_digest": RESOURCE_CEILING_DIGEST,
        "provenance": {
            "owner_revision": SOURCE_REVISION,
            "descriptor_semantic_digest": descriptor["descriptor_digest"],
            "descriptor_exact_checksum": digest_bytes(descriptor_bytes),
            "dirty": False,
        },
        "fail_closed_on": ["missing", "placeholder", "dirty", "mismatched", "implicit-default"],
    }
    (runtime_root / "startup-input.json").write_bytes(json_bytes(payload))
    os.chmod(runtime_root, 0o755)
    return payload


def lifecycle_probe(image: str, runtime_root: Path) -> dict[str, Any]:
    result = run(
        [
            "docker",
            "run",
            "--rm",
            "--platform=linux/arm64",
            "--mount",
            f"type=bind,src={runtime_root},dst=/runtime,readonly",
            image,
            "--startup-input",
            "/runtime/startup-input.json",
            "--lifecycle-probe",
            "drain_shutdown_after_in_flight_completion",
        ]
    )
    lines = [line for line in result.stdout.splitlines() if line.strip()]
    require(len(lines) == 1, "lifecycle probe did not emit exactly one result")
    probe = json.loads(lines[0])
    require(isinstance(probe, dict), "lifecycle probe result is not an object")
    return probe


def run_lifecycle(image: str, temporary_root: Path) -> dict[str, Any]:
    lifecycle = load_module(LIFECYCLE_PATH, "reference_host_lifecycle_for_image")
    runtime_root = temporary_root / "runtime"
    runtime_root.mkdir()
    startup = startup_input(runtime_root)
    _, invoke_vector, lifecycle_vector = lifecycle.load_profile_and_vectors()
    minimal_air = copy.deepcopy(invoke_vector["fixtures"]["minimal_valid_air"])
    invalid_air = copy.deepcopy(invoke_vector["fixtures"]["invalid_air_missing_source_map"])

    cases: list[dict[str, Any]] = []
    for case_name, runner in lifecycle.executed_case_runners():
        try:
            case = runner(
                executable_path=Path("/usr/local/bin/apxm-reference-host"),
                startup_input_path=runtime_root / "startup-input.json",
                startup_input=startup,
                minimal_air=minimal_air,
                invalid_air=invalid_air,
                transport_factory=lambda _executable, _startup: DockerTransport(image, runtime_root),
                lifecycle_probe_runner=lambda _executable, _startup: lifecycle_probe(image, runtime_root),
            )
        except Exception as error:
            raise ImageGateError(f"lifecycle case {case_name} failed: {error}") from error
        require(case.get("status") == "passed", f"lifecycle case {case_name} did not pass")
        cases.append(case)

    release = json.loads(git_file(SOURCE_REVISION, str(RELEASE_MANIFEST_PATH.relative_to(ROOT))))
    required = release["executable_parity_evidence"]["live_required_cases"]
    executed = [case["name"] for case in cases]
    require(set(required).issubset(executed), "image lifecycle did not cover every live-required case")
    return {
        "harness_path": "crates/tools/cli/tests/reference_host_jsonl.rs",
        "harness_digest": digest_bytes(
            git_file(SOURCE_REVISION, "crates/tools/cli/tests/reference_host_jsonl.rs")
        ),
        "invoke_vector_digest": digest_bytes(
            git_file(
                SOURCE_REVISION,
                "contracts/reference-host/vectors/apxm.reference-host.invoke-parity.v1.json",
            )
        ),
        "lifecycle_vector_digest": digest_bytes(
            git_file(
                SOURCE_REVISION,
                "contracts/reference-host/vectors/apxm.reference-host.lifecycle-parity.v1.json",
            )
        ),
        "required_cases": required,
        "executed_cases": executed,
        "passed_case_count": len(cases),
        "outcomes": lifecycle.lifecycle_outcomes_summary(cases),
    }


def build_receipt(gate_revision: str) -> dict[str, Any]:
    image_checker = load_module(IMAGE_CHECKER_PATH, "reference_host_image_checker_for_gate")
    image_facts = image_checker.validate(require_clean=True)
    gate_files = exact_gate_files(gate_revision)
    with tempfile.TemporaryDirectory(prefix="image-gate-", dir=ROOT / ".apxm") as temp_name:
        temporary_root = Path(temp_name)
        materialize_build_context(temporary_root)
        context = temporary_root / "context"
        tag_one = f"apxm-reference-host:gate-{gate_revision[:12]}-one"
        tag_two = f"apxm-reference-host:gate-{gate_revision[:12]}-two"
        first_id = build_image(context, tag_one)
        second_id = build_image(context, tag_two)
        require(first_id == second_id, "two no-cache builds produced different OCI image IDs")
        inspection = inspect_image(tag_two)
        require(inspection["image_id"] == first_id, "inspected image differs from reproducible build")
        binary = extract_binary(tag_two, temporary_root / "apxm-reference-host")
        lifecycle = run_lifecycle(tag_two, temporary_root)

    image_manifest_bytes = IMAGE_MANIFEST_PATH.read_bytes()
    return {
        "schema_version": "apxm.reference-host-image-build-receipt.v1",
        "semantic_owner": "agents",
        "status": "complete",
        "source_revision": SOURCE_REVISION,
        "reviewed_carrier_revision": REVIEWED_CARRIER_REVISION,
        "recipe_revision": RECIPE_REVISION,
        "gate_revision": gate_revision,
        "gate_files": gate_files,
        "image_manifest": {
            "path": "deploy/reference-host/image-manifest.v1.json",
            "semantic_digest": image_facts["manifest_digest"],
            "exact_bytes_digest": digest_bytes(image_manifest_bytes),
        },
        "build": {
            "platform": "linux/arm64",
            "no_cache": True,
            "provenance": False,
            "run_count": 2,
            "image_ids": [first_id, second_id],
            "reproducible": True,
        },
        "inspection": inspection,
        "binary": binary,
        "lifecycle": lifecycle,
    }


def validate_receipt_payload(receipt: dict[str, Any]) -> None:
    require(receipt.get("schema_version") == "apxm.reference-host-image-build-receipt.v1", "receipt schema drifted")
    require(receipt.get("semantic_owner") == "agents", "receipt owner drifted")
    require(receipt.get("status") == "complete", "receipt is not complete")
    require(receipt.get("source_revision") == SOURCE_REVISION, "receipt source revision drifted")
    require(receipt.get("reviewed_carrier_revision") == REVIEWED_CARRIER_REVISION, "receipt reviewed carrier drifted")
    require(receipt.get("recipe_revision") == RECIPE_REVISION, "receipt recipe revision drifted")
    gate_revision = receipt.get("gate_revision")
    require(isinstance(gate_revision, str) and len(gate_revision) == 40, "receipt gate revision is invalid")
    require(receipt.get("gate_files") == exact_gate_files(gate_revision), "receipt gate files drifted")
    image_ids = receipt.get("build", {}).get("image_ids")
    require(isinstance(image_ids, list) and len(image_ids) == 2, "receipt lacks two build image IDs")
    require(image_ids[0] == image_ids[1], "receipt does not prove reproducible image IDs")
    require(receipt.get("build", {}).get("run_count") == 2, "receipt build count drifted")
    require(receipt.get("build", {}).get("no_cache") is True, "receipt did not use no-cache builds")
    require(receipt.get("build", {}).get("provenance") is False, "receipt did not disable variable provenance")
    inspection = receipt.get("inspection", {})
    require(inspection.get("image_id") == image_ids[0], "receipt inspected a different image")
    require(inspection.get("os") == "linux" and inspection.get("architecture") == "arm64", "receipt platform drifted")
    require(inspection.get("user") == "65532:65532", "receipt runtime User drifted")
    require(inspection.get("entrypoint") == ["/usr/local/bin/apxm-reference-host"], "receipt entrypoint drifted")
    require(inspection.get("labels") == expected_labels(), "receipt labels drifted")
    binary = receipt.get("binary", {})
    require(binary.get("elf_class") == "ELF64", "receipt ELF class drifted")
    require(binary.get("elf_type") == "PIE", "receipt ELF type drifted")
    require(binary.get("elf_machine") == "AArch64", "receipt ELF machine drifted")
    require(isinstance(binary.get("exact_bytes_digest"), str), "receipt binary digest is missing")
    lifecycle = receipt.get("lifecycle", {})
    required = lifecycle.get("required_cases")
    executed = lifecycle.get("executed_cases")
    require(isinstance(required, list) and isinstance(executed, list), "receipt lifecycle cases are missing")
    require(set(required).issubset(executed), "receipt lifecycle coverage is incomplete")
    require(lifecycle.get("passed_case_count") == len(executed), "receipt lifecycle pass count drifted")


def write_receipt(receipt: dict[str, Any]) -> None:
    RECEIPT_PATH.parent.mkdir(parents=True, exist_ok=True)
    RECEIPT_PATH.write_bytes(json_bytes(receipt))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    actions = parser.add_mutually_exclusive_group(required=True)
    actions.add_argument("--write", action="store_true")
    actions.add_argument("--check", action="store_true")
    args = parser.parse_args()
    try:
        require(not git_text("status", "--porcelain=v1", "--untracked-files=all"), "owner checkout is dirty")
        if args.write:
            gate_revision = git_text("rev-parse", "HEAD")
        else:
            try:
                committed = json.loads(RECEIPT_PATH.read_text(encoding="utf-8"))
            except (OSError, json.JSONDecodeError) as error:
                raise ImageGateError(f"cannot load committed machine receipt: {error}") from error
            validate_receipt_payload(committed)
            gate_revision = committed["gate_revision"]
        observed = build_receipt(gate_revision)
        validate_receipt_payload(observed)
        if args.write:
            write_receipt(observed)
        else:
            require(observed == committed, "committed machine receipt differs from dynamic observations")
    except (ImageGateError, KeyError, OSError, json.JSONDecodeError) as error:
        print(f"FAIL: {error}")
        return 1
    print("PASS: Agents reference-host dynamic Linux/arm64 image gate")
    print(f"image_id: {observed['inspection']['image_id']}")
    print(f"binary_digest: {observed['binary']['exact_bytes_digest']}")
    print(f"lifecycle_cases: {observed['lifecycle']['passed_case_count']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
