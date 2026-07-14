#!/usr/bin/env python3
"""Execute preregistered prompt arms and emit portable backend evidence."""

from __future__ import annotations

import argparse
import datetime as dt
import importlib.util
import json
import os
import shutil
import ssl
import subprocess
import sys
import time
import tomllib
import urllib.error
import urllib.request
from dataclasses import dataclass
from pathlib import Path
from typing import Any
from urllib.parse import urlsplit

from apxm.contract import build_layout


EVIDENCE_SCRIPT = Path(__file__).with_name("offline_prompt_evaluation.py")
EVIDENCE_SPEC = importlib.util.spec_from_file_location("offline_prompt_evaluation", EVIDENCE_SCRIPT)
if EVIDENCE_SPEC is None or EVIDENCE_SPEC.loader is None:
    raise RuntimeError("unable to load offline prompt evaluation contract")
evidence = importlib.util.module_from_spec(EVIDENCE_SPEC)
sys.modules.setdefault(EVIDENCE_SPEC.name, evidence)
EVIDENCE_SPEC.loader.exec_module(evidence)


SUPPORTED_PROTOCOLS = frozenset({"anthropic", "openai", "vllm"})
SENSITIVE_HEADER_NAMES = frozenset(
    {
        "authorization",
        "api-key",
        "x-api-key",
        "ocp-apim-subscription-key",
        "x-goog-api-key",
    }
)
ARM_FIELDS = frozenset({"arm_id", "system_prompt", "user_template", "temperature", "max_tokens"})
HELD_OUT_FIELDS = frozenset({"id", "input", "expected"})


@dataclass(frozen=True)
class BackendRegistration:
    name: str
    protocol: str
    endpoint: str
    api_key: str
    headers: dict[str, str]
    model: str
    capabilities: dict[str, Any]


@dataclass(frozen=True)
class ArmTemplate:
    arm_id: str
    system_prompt: str
    user_template: str
    temperature: float
    max_tokens: int


@dataclass(frozen=True)
class BackendObservation:
    response: dict[str, Any]
    output: str
    token_count: int
    provider_response_id: str
    provider_model: str


def require_exact_keys(value: dict[str, Any], name: str, expected: frozenset[str]) -> None:
    missing = sorted(expected - value.keys())
    unexpected = sorted(value.keys() - expected)
    if missing:
        raise ValueError(f"{name} missing {missing}")
    if unexpected:
        raise ValueError(f"{name} has unexpected keys {unexpected}")


def resolve_reference(value: Any, name: str, *, secret: bool = False) -> str:
    reference = evidence.require_non_empty_string(value, name)
    if reference.startswith("env:"):
        variable = reference.removeprefix("env:")
        resolved = os.environ.get(variable)
        if not resolved:
            raise ValueError(f"{name} references unset environment variable {variable!r}")
        return resolved
    if secret:
        raise ValueError(f"{name} must be an env: reference")
    return reference


def normalize_endpoint(protocol: str, endpoint: str) -> str:
    trimmed = endpoint.rstrip("/")
    parsed = urlsplit(trimmed)
    if parsed.scheme not in {"http", "https"} or not parsed.netloc:
        raise ValueError("backend endpoint must be an absolute HTTP(S) URL")
    if protocol in {"anthropic", "openai", "vllm"} and not trimmed.endswith("/v1"):
        if parsed.path in {"", "/"}:
            trimmed += "/v1"
    if protocol == "anthropic" and trimmed.endswith("/Anthropic"):
        trimmed += "/v1"
    return trimmed


def model_capabilities(backend: dict[str, Any], model: dict[str, Any]) -> dict[str, Any]:
    return {
        "protocol": evidence.require_identifier(
            backend.get("protocol"), "configured backend protocol"
        ),
        "model": evidence.require_identifier(model.get("id"), "configured backend model"),
        "context_window": evidence.require_non_negative_int(
            model.get("context_window", 0), "configured backend context_window"
        ),
        "supports_functions": bool(model.get("supports_functions", False)),
        "supports_vision": bool(model.get("supports_vision", False)),
        "supports_thinking": bool(model.get("supports_thinking", False)),
        "supports_structured_outputs": bool(
            model.get(
                "supports_structured_outputs",
                backend.get("supports_structured_outputs", False),
            )
        ),
        "source": "registered-backend",
    }


def load_backend(
    config_path: Path,
    backend_name: str,
    preregistered: dict[str, Any],
) -> BackendRegistration:
    config = tomllib.loads(config_path.read_text(encoding="utf-8"))
    matches = [item for item in config.get("backends", []) if item.get("name") == backend_name]
    if len(matches) != 1:
        raise ValueError(f"configured backend {backend_name!r} must resolve exactly once")
    backend = matches[0]
    protocol = evidence.require_identifier(backend.get("protocol"), "configured backend protocol")
    if protocol not in SUPPORTED_PROTOCOLS:
        raise ValueError(
            f"observed prompt evaluation does not support configured protocol {protocol!r}"
        )
    models = backend.get("models")
    if not isinstance(models, list) or not models:
        raise ValueError("configured backend must declare at least one model")
    preregistered_backend = preregistered["backend"]
    if preregistered_backend["id"] != backend_name:
        raise ValueError("selected backend does not match preregistered backend id")
    model = next(
        (item for item in models if item.get("id") == preregistered_backend["revision"]),
        None,
    )
    if not isinstance(model, dict):
        raise ValueError("preregistered backend revision must identify a configured model")
    capabilities = model_capabilities(backend, model)
    if capabilities != preregistered["capabilities"]:
        raise ValueError("configured backend capabilities do not match preregistration")

    headers: dict[str, str] = {}
    for name, value in backend.get("headers", {}).items():
        if not isinstance(name, str):
            raise ValueError("configured backend header names must be strings")
        headers[name] = resolve_reference(
            value,
            f"configured backend header {name!r}",
            secret=name.lower() in SENSITIVE_HEADER_NAMES,
        )
    return BackendRegistration(
        name=backend_name,
        protocol=protocol,
        endpoint=normalize_endpoint(
            protocol,
            resolve_reference(backend.get("endpoint"), "configured backend endpoint"),
        ),
        api_key=resolve_reference(
            backend.get("api_key"), "configured backend api_key", secret=True
        ),
        headers=headers,
        model=capabilities["model"],
        capabilities=capabilities,
    )


def load_arm(path: Path, expected_arm_id: str) -> ArmTemplate:
    value = evidence.load_json(path)
    require_exact_keys(value, f"arm template {path.name}", ARM_FIELDS)
    arm_id = evidence.require_identifier(value.get("arm_id"), f"{path.name} arm_id")
    if arm_id != expected_arm_id:
        raise ValueError(f"{path.name} arm_id does not match preregistration")
    system_prompt = evidence.require_non_empty_string(
        value.get("system_prompt"), f"{path.name} system_prompt"
    )
    user_template = evidence.require_non_empty_string(
        value.get("user_template"), f"{path.name} user_template"
    )
    if user_template.count("{input}") != 1:
        raise ValueError(f"{path.name} user_template must contain exactly one {{input}} placeholder")
    temperature = evidence.require_number(
        value.get("temperature"), f"{path.name} temperature", 0.0, 2.0
    )
    max_tokens = evidence.require_non_negative_int(
        value.get("max_tokens"), f"{path.name} max_tokens"
    )
    if max_tokens == 0:
        raise ValueError(f"{path.name} max_tokens must be positive")
    return ArmTemplate(arm_id, system_prompt, user_template, temperature, max_tokens)


def load_held_out(path: Path) -> dict[str, dict[str, str]]:
    rows = evidence.load_jsonl(path, set(HELD_OUT_FIELDS))
    validated: dict[str, dict[str, str]] = {}
    for case_id, row in rows.items():
        require_exact_keys(row, f"held-out case {case_id!r}", HELD_OUT_FIELDS)
        validated[case_id] = {
            "id": case_id,
            "input": evidence.require_non_empty_string(
                row.get("input"), f"held-out input for {case_id!r}"
            ),
            "expected": evidence.require_non_empty_string(
                row.get("expected"), f"held-out expected for {case_id!r}"
            ),
        }
    return validated


def request_payload(
    backend: BackendRegistration, arm: ArmTemplate, case: dict[str, str]
) -> dict[str, Any]:
    user_prompt = arm.user_template.replace("{input}", case["input"])
    if backend.protocol == "anthropic":
        return {
            "model": backend.model,
            "system": arm.system_prompt,
            "messages": [{"role": "user", "content": user_prompt}],
            "temperature": arm.temperature,
            "max_tokens": arm.max_tokens,
        }
    return {
        "model": backend.model,
        "messages": [
            {"role": "system", "content": arm.system_prompt},
            {"role": "user", "content": user_prompt},
        ],
        "temperature": arm.temperature,
        "max_tokens": arm.max_tokens,
    }


def request_url(backend: BackendRegistration) -> str:
    path = "/messages" if backend.protocol == "anthropic" else "/chat/completions"
    return backend.endpoint + path


def request_headers(backend: BackendRegistration) -> dict[str, str]:
    headers = {"content-type": "application/json", **backend.headers}
    if backend.protocol == "anthropic":
        headers["x-api-key"] = backend.api_key
        headers["anthropic-version"] = "2023-06-01"
    elif backend.api_key:
        headers["authorization"] = f"Bearer {backend.api_key}"
    return headers


def trusted_ssl_context() -> ssl.SSLContext:
    default_paths = ssl.get_default_verify_paths()
    candidates = [
        os.environ.get("SSL_CERT_FILE"),
        default_paths.cafile,
        "/etc/ssl/certs/ca-certificates.crt",
        "/etc/pki/tls/certs/ca-bundle.crt",
    ]
    for candidate in candidates:
        if candidate and Path(candidate).is_file():
            return ssl.create_default_context(cafile=candidate)
    if default_paths.capath and Path(default_paths.capath).is_dir():
        return ssl.create_default_context(capath=default_paths.capath)
    raise ValueError("no trusted TLS certificate store is available")


def perform_request(
    backend: BackendRegistration,
    payload: dict[str, Any],
    timeout_seconds: float,
) -> tuple[dict[str, Any], float]:
    encoded = json.dumps(payload, ensure_ascii=True, separators=(",", ":")).encode("utf-8")
    request = urllib.request.Request(
        request_url(backend),
        data=encoded,
        headers=request_headers(backend),
        method="POST",
    )
    started = time.monotonic()
    try:
        with urllib.request.urlopen(
            request,
            timeout=timeout_seconds,
            context=trusted_ssl_context(),
        ) as response:
            body = response.read()
    except urllib.error.HTTPError as error:
        detail = error.read().decode("utf-8", errors="replace")
        raise ValueError(f"backend request failed with HTTP {error.code}: {detail}") from error
    except urllib.error.URLError as error:
        raise ValueError(f"backend request failed: {error.reason}") from error
    latency_ms = round((time.monotonic() - started) * 1000.0, 3)
    value = json.loads(body)
    if not isinstance(value, dict):
        raise ValueError("backend response must be a JSON object")
    return value, latency_ms


def parse_observation(backend: BackendRegistration, response: dict[str, Any]) -> BackendObservation:
    provider_response_id = evidence.require_identifier(
        response.get("id"), "backend response id"
    )
    provider_model = evidence.require_identifier(
        response.get("model", backend.model), "backend response model"
    )
    if backend.protocol == "anthropic":
        content = response.get("content")
        if not isinstance(content, list):
            raise ValueError("Anthropic response content must be an array")
        output = "".join(
            block.get("text", "")
            for block in content
            if isinstance(block, dict) and block.get("type") == "text"
        ).strip()
        usage = evidence.require_mapping(response.get("usage"), "Anthropic response usage")
        token_count = evidence.require_non_negative_int(
            usage.get("input_tokens"), "Anthropic input_tokens"
        ) + evidence.require_non_negative_int(
            usage.get("output_tokens"), "Anthropic output_tokens"
        )
    else:
        choices = response.get("choices")
        if not isinstance(choices, list) or not choices or not isinstance(choices[0], dict):
            raise ValueError("OpenAI-compatible response choices must be a non-empty array")
        message = evidence.require_mapping(
            choices[0].get("message"), "OpenAI-compatible response message"
        )
        output = evidence.require_non_empty_string(
            message.get("content"), "OpenAI-compatible response content"
        ).strip()
        usage = evidence.require_mapping(
            response.get("usage"), "OpenAI-compatible response usage"
        )
        total_tokens = usage.get("total_tokens")
        if total_tokens is None:
            total_tokens = evidence.require_non_negative_int(
                usage.get("prompt_tokens"), "OpenAI-compatible prompt_tokens"
            ) + evidence.require_non_negative_int(
                usage.get("completion_tokens"), "OpenAI-compatible completion_tokens"
            )
        token_count = evidence.require_non_negative_int(
            total_tokens, "OpenAI-compatible total_tokens"
        )
    if not output:
        raise ValueError("backend response output must not be empty")
    return BackendObservation(
        response=response,
        output=output,
        token_count=token_count,
        provider_response_id=provider_response_id,
        provider_model=provider_model,
    )


def write_json(path: Path, value: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def write_jsonl(path: Path, rows: list[dict[str, Any]]) -> None:
    path.write_text(
        "".join(json.dumps(row, ensure_ascii=True, sort_keys=True) + "\n" for row in rows),
        encoding="utf-8",
    )


def git_text(root: Path, *args: str) -> str:
    try:
        result = subprocess.run(
            ["git", *args],
            cwd=root,
            check=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
    except (FileNotFoundError, subprocess.CalledProcessError) as error:
        detail = getattr(error, "stderr", "").strip()
        suffix = f": {detail}" if detail else ""
        raise ValueError(f"unable to verify preregistration history{suffix}") from error
    return result.stdout.strip()


def committed_preregistration_evidence(
    repo_root: Path,
    preregistration_path: Path,
    declared_revision: str,
) -> dict[str, str]:
    relative_path = evidence.portable_path(
        preregistration_path, repo_root, "preregistration source"
    )
    git_text(repo_root, "ls-files", "--error-unmatch", "--", relative_path)
    status = git_text(repo_root, "status", "--porcelain", "--", relative_path)
    if status:
        raise ValueError("claim-bearing preregistration must be committed and unmodified")
    source_commit = evidence.require_revision(
        git_text(repo_root, "log", "-1", "--format=%H", "--", relative_path),
        "preregistration source commit",
    )
    evaluated_head = evidence.require_revision(
        git_text(repo_root, "rev-parse", "--verify", "HEAD"), "evaluated HEAD revision"
    )
    for revision, label in (
        (declared_revision, "declared provenance revision"),
        (source_commit, "preregistration source commit"),
    ):
        try:
            subprocess.run(
                ["git", "merge-base", "--is-ancestor", revision, evaluated_head],
                cwd=repo_root,
                check=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
            )
        except subprocess.CalledProcessError as error:
            raise ValueError(f"{label} must be an ancestor of evaluated HEAD") from error
    return {
        "repository": "apxm-project/agents",
        "source_path": relative_path,
        "source_commit": source_commit,
        "declared_revision": declared_revision,
        "evaluated_head": evaluated_head,
        "preregistration_sha256": evidence.sha256_file(preregistration_path),
    }


def record_observation(
    bundle_root: Path,
    backend_evidence: dict[str, Any],
    arm: ArmTemplate,
    case: dict[str, str],
    request: dict[str, Any],
    observation: BackendObservation,
    latency_ms: float,
) -> dict[str, Any]:
    stem = f"{arm.arm_id}-{case['id']}"
    request_path = bundle_root / "requests" / f"{stem}.json"
    response_path = bundle_root / "responses" / f"{stem}.json"
    write_json(request_path, request)
    write_json(response_path, observation.response)
    request_reference = evidence.file_reference(request_path, bundle_root, "observed request")
    response_reference = evidence.file_reference(response_path, bundle_root, "observed response")
    execution = {
        "schema_version": evidence.OBSERVED_BACKEND_EXECUTION_SCHEMA_VERSION,
        "kind": "recorded-backend-execution",
        "backend": backend_evidence["backend"],
        "capabilities_sha256": evidence.sha256_json(backend_evidence["capabilities"]),
        "case_id": case["id"],
        "arm_id": arm.arm_id,
        "request": request_reference,
        "response": response_reference,
        "provider_response_id": observation.provider_response_id,
        "provider_model": observation.provider_model,
        "output_sha256": evidence.sha256_text(observation.output),
        "token_count": observation.token_count,
        "latency_ms": latency_ms,
        "observed_at_utc": dt.datetime.now(dt.UTC).isoformat(timespec="milliseconds").replace(
            "+00:00", "Z"
        ),
    }
    execution_path = bundle_root / "executions" / f"{stem}.json"
    write_json(execution_path, execution)
    execution_reference = evidence.file_reference(
        execution_path, bundle_root, "observed execution"
    )
    receipt = {
        "schema_version": evidence.OBSERVED_BACKEND_RECEIPT_SCHEMA_VERSION,
        "kind": "observed-backend-receipt",
        "backend": backend_evidence["backend"],
        "case_id": case["id"],
        "arm_id": arm.arm_id,
        "request_sha256": request_reference["sha256"],
        "output_sha256": evidence.sha256_text(observation.output),
        "token_count": observation.token_count,
        "latency_ms": latency_ms,
        "execution_evidence": execution_reference,
    }
    receipt_path = bundle_root / "receipts" / f"{stem}.json"
    write_json(receipt_path, receipt)
    return {
        "id": case["id"],
        "output": observation.output,
        "arm_id": arm.arm_id,
        "token_count": observation.token_count,
        "latency_ms": latency_ms,
        "evidence": evidence.file_reference(receipt_path, bundle_root, "observed receipt"),
    }


def copy_input(path: Path, bundle_root: Path) -> Path:
    destination = bundle_root / path.name
    shutil.copy2(path, destination)
    return destination


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--preregistration", type=Path, required=True)
    parser.add_argument("--optimization", type=Path, required=True)
    parser.add_argument("--held-out", type=Path, required=True)
    parser.add_argument("--baseline-arm", type=Path, required=True)
    parser.add_argument("--candidate-arm", type=Path, required=True)
    parser.add_argument("--backend", required=True)
    parser.add_argument("--config", type=Path, default=Path.home() / ".apxm" / "config.toml")
    parser.add_argument("--run-id")
    parser.add_argument("--timeout-seconds", type=float, default=120.0)
    return parser.parse_args(argv)


def main(argv: list[str]) -> int:
    args = parse_args(argv)
    preregistration = evidence.validate_preregistration(evidence.load_json(args.preregistration))
    backend_evidence = preregistration["backend_evidence"]
    if backend_evidence["kind"] != "observed-backend-run":
        raise ValueError("observed prompt evaluation requires observed-backend-run evidence")
    if evidence.sha256_file(args.optimization) != preregistration["splits"]["optimization"][
        "sha256"
    ]:
        raise ValueError("optimization input digest does not match preregistration")
    if evidence.sha256_file(args.held_out) != preregistration["splits"]["held_out"]["sha256"]:
        raise ValueError("held-out input digest does not match preregistration")
    held_out = load_held_out(args.held_out)
    if len(held_out) != preregistration["splits"]["held_out"]["case_count"]:
        raise ValueError("held-out input case count does not match preregistration")
    baseline_arm = load_arm(args.baseline_arm, preregistration["arms"]["baseline"]["id"])
    candidate_arm = load_arm(args.candidate_arm, preregistration["arms"]["candidate"]["id"])
    backend = load_backend(args.config, args.backend, backend_evidence)
    if args.timeout_seconds <= 0:
        raise ValueError("timeout-seconds must be positive")

    layout = build_layout(__file__)
    preregistration_source = committed_preregistration_evidence(
        layout.repo_root,
        args.preregistration,
        preregistration["provenance"]["revision"],
    )
    run_id = args.run_id or evidence.utc_run_id()
    evidence.require_identifier(run_id, "run id")
    bundle_root = layout.evaluation_dir / preregistration["scenario"] / "bundles" / run_id
    if bundle_root.exists():
        raise ValueError(f"observed bundle directory already exists: {bundle_root}")
    bundle_root.mkdir(parents=True)
    copied = {
        "preregistration": copy_input(args.preregistration, bundle_root),
        "optimization": copy_input(args.optimization, bundle_root),
        "held_out": copy_input(args.held_out, bundle_root),
        "baseline_arm": copy_input(args.baseline_arm, bundle_root),
        "candidate_arm": copy_input(args.candidate_arm, bundle_root),
    }
    preregistration_source_path = bundle_root / "preregistration-source.json"
    write_json(preregistration_source_path, preregistration_source)

    outputs: dict[str, list[dict[str, Any]]] = {"baseline": [], "candidate": []}
    for label, arm in (("baseline", baseline_arm), ("candidate", candidate_arm)):
        for case_id in sorted(held_out):
            case = held_out[case_id]
            payload = request_payload(backend, arm, case)
            response, latency_ms = perform_request(backend, payload, args.timeout_seconds)
            observation = parse_observation(backend, response)
            outputs[label].append(
                record_observation(
                    bundle_root,
                    backend_evidence,
                    arm,
                    case,
                    payload,
                    observation,
                    latency_ms,
                )
            )
    baseline_path = bundle_root / "baseline.jsonl"
    candidate_path = bundle_root / "candidate.jsonl"
    write_jsonl(baseline_path, outputs["baseline"])
    write_jsonl(candidate_path, outputs["candidate"])

    result = evidence.main(
        [
            "--preregistration",
            str(copied["preregistration"]),
            "--bundle-root",
            str(bundle_root),
            "--optimization",
            str(copied["optimization"]),
            "--held-out",
            str(copied["held_out"]),
            "--baseline",
            str(baseline_path),
            "--candidate",
            str(candidate_path),
            "--run-id",
            run_id,
        ]
    )
    if result != 0:
        return result
    run_dir = layout.evaluation_dir / preregistration["scenario"] / "runs" / run_id
    manifest_path = run_dir / "manifest.json"
    manifest = evidence.load_json(manifest_path)
    manifest["observed_bundle"] = {
        "path": evidence.portable_path(bundle_root, layout.repo_root, "observed bundle directory"),
        "preregistration_source": evidence.file_reference(
            preregistration_source_path,
            bundle_root,
            "preregistration source evidence",
        ),
        "runner_sha256": evidence.sha256_file(Path(__file__)),
    }
    write_json(manifest_path, manifest)
    print(
        json.dumps(
            {
                "bundle_dir": evidence.portable_path(
                    bundle_root, layout.repo_root, "observed bundle directory"
                ),
                "run_id": run_id,
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main(sys.argv[1:]))
    except (OSError, ValueError, json.JSONDecodeError, tomllib.TOMLDecodeError) as error:
        print(f"observed prompt evaluation failed: {error}", file=sys.stderr)
        raise SystemExit(2)
