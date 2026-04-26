"""DSPy optimizer — configure LM, run optimizer, extract results."""

from __future__ import annotations

import os
import json
from pathlib import Path
from typing import Any

from .cache import compute_cache_key, hash_training_data, load_cached, store_cached
from .metrics import build_metric
from .signatures import extract_fields_from_template
from .types import (
    DEFAULT_DSPY_PREFIX,
    ENV_VALUE_PREFIX,
    AutoLevel,
    BackendKey,
    Metric,
    Optimizer,
    RequestKey,
    ResponseKey,
    Protocol,
    Status,
    PROTOCOL_TO_DSPY_PREFIX,
)


def _resolve_env(value: str, default: str = "") -> str:
    """Resolve 'env:VAR_NAME' to its environment variable value."""
    if isinstance(value, str) and value.startswith(ENV_VALUE_PREFIX):
        return os.environ.get(value[len(ENV_VALUE_PREFIX) :], default)
    return value


def _dspy() -> Any:
    import dspy

    return dspy


def _load_training_data(request: dict) -> list[dict]:
    if RequestKey.TRAINING_DATA in request:
        data = request.get(RequestKey.TRAINING_DATA, [])
        if isinstance(data, list):
            return data
        raise ValueError("DSPy training_data must be a list")

    path = request.get(RequestKey.TRAINING_DATA_PATH)
    if not path:
        return []
    training_path = Path(str(path))
    with training_path.open() as handle:
        data = json.load(handle)
    if not isinstance(data, list):
        raise ValueError("DSPy training_data_path must contain a JSON list")
    return data


def _load_backend(request: dict) -> dict:
    backend = request.get(RequestKey.BACKEND, {})
    if isinstance(backend, dict) and backend:
        return backend

    backend_json = request.get(RequestKey.BACKEND_JSON)
    if backend_json:
        parsed = json.loads(str(backend_json))
        if not isinstance(parsed, dict):
            raise ValueError("DSPy backend_json must decode to an object")
        return parsed
    return {}


def _backend_fingerprint(backend: dict) -> str:
    public_backend = {
        key: backend.get(key.value)
        for key in (BackendKey.PROTOCOL, BackendKey.MODEL, BackendKey.ENDPOINT)
    }
    return json.dumps(public_backend, sort_keys=True, ensure_ascii=True)


def configure_lm(backend: dict):
    """Map APXM backend config to dspy.LM()."""
    dspy = _dspy()
    protocol = backend.get(BackendKey.PROTOCOL, Protocol.OPENAI)
    try:
        proto = Protocol(protocol)
        prefix = PROTOCOL_TO_DSPY_PREFIX.get(proto, DEFAULT_DSPY_PREFIX)
    except ValueError:
        prefix = DEFAULT_DSPY_PREFIX
    model = backend.get(BackendKey.MODEL)
    if not model:
        raise ValueError("DSPy backend config requires a registered model")

    api_key = _resolve_env(backend.get(BackendKey.API_KEY, ""))
    endpoint = backend.get(BackendKey.ENDPOINT)
    resolved_headers = {
        k: _resolve_env(v, v) for k, v in backend.get(BackendKey.HEADERS, {}).items()
    }

    kwargs = {
        "api_key": api_key,
    }
    if endpoint:
        kwargs["api_base"] = endpoint
    if resolved_headers:
        kwargs["extra_headers"] = resolved_headers

    return dspy.LM(f"{prefix}/{model}", **kwargs)


def optimize_single_template(
    template_str: str,
    training_data: list[dict],
    cache_dir: Path,
    optimizer_name: str,
    auto: str,
    metric_name: str,
    model: str = "",
    backend_fingerprint: str = "",
    dspy_version: str = "",
    no_cache: bool = False,
) -> dict:
    """Optimize a single template using DSPy."""
    dspy = _dspy()
    # Check cache first
    td_hash = hash_training_data(training_data)
    cache_key = compute_cache_key(
        template_str,
        td_hash,
        optimizer_name,
        model,
        metric_name,
        auto,
        backend_fingerprint,
        dspy_version,
    )
    cached = load_cached(cache_key, cache_dir, disabled=no_cache)
    if cached is not None:
        cached[ResponseKey.CACHE_HIT] = True
        return cached

    input_fields, output_field = extract_fields_from_template(template_str, training_data)

    signature_str = ", ".join(input_fields) + " -> " + output_field
    predictor = dspy.Predict(signature_str)

    examples = []
    for ex in training_data:
        fields = dict(ex.get("inputs", {}))
        fields[output_field] = ex.get("output", "")
        examples.append(dspy.Example(**fields).with_inputs(*input_fields))

    metric_fn = build_metric(metric_name, output_field)

    if optimizer_name == Optimizer.BOOTSTRAP:
        optimizer = dspy.BootstrapFewShot(metric=metric_fn, max_bootstrapped_demos=4)
        compiled = optimizer.compile(predictor, trainset=examples)
    elif optimizer_name == Optimizer.COPRO:
        optimizer = dspy.COPRO(metric=metric_fn)
        compiled = optimizer.compile(predictor, trainset=examples)
    else:
        # Default and Optimizer.MIPRO
        optimizer = dspy.MIPROv2(metric=metric_fn, auto=auto)
        compiled = optimizer.compile(
            predictor,
            trainset=examples,
            max_bootstrapped_demos=0,
            max_labeled_demos=0,
        )

    optimized_instruction = compiled.signature.instructions

    # Reconstruct template preserving named `{name}` placeholders
    optimized_template = optimized_instruction + "\n\n" + template_str

    result = {
        ResponseKey.OPTIMIZED_TEMPLATE: optimized_template,
        ResponseKey.OPTIMIZED_INSTRUCTIONS: optimized_instruction,
        ResponseKey.ORIGINAL_TEMPLATE: template_str,
        ResponseKey.ORIGINAL_TEMPLATE_CHARS: len(template_str),
        ResponseKey.OPTIMIZED_TEMPLATE_CHARS: len(optimized_template),
        ResponseKey.TEMPLATE_CHAR_DELTA: len(optimized_template) - len(template_str),
        ResponseKey.OPTIMIZED_INSTRUCTION_CHARS: len(optimized_instruction),
        ResponseKey.TRAINING_EXAMPLES: len(training_data),
        ResponseKey.CACHE_HIT: False,
    }
    store_cached(cache_key, result, cache_dir, disabled=no_cache)
    return result


def optimize_templates(request: dict) -> dict:
    """Process a batch request to optimize one or more templates."""
    dspy = _dspy()
    backend = _load_backend(request)
    lm = configure_lm(backend)
    dspy.configure(lm=lm)

    training_data = _load_training_data(request)
    cache_root = request.get(RequestKey.CACHE_DIR)
    if not cache_root:
        raise ValueError("DSPy compiler request requires cache_dir")
    cache_dir = Path(str(cache_root))
    optimizer_name = request.get(RequestKey.OPTIMIZER, Optimizer.MIPRO)
    auto = request.get(RequestKey.AUTO, AutoLevel.LIGHT)
    metric_name = request.get(RequestKey.METRIC, Metric.TOKEN_OVERLAP)
    model = backend.get(BackendKey.MODEL, "")
    no_cache = bool(request.get(RequestKey.NO_CACHE, False))
    backend_fingerprint = _backend_fingerprint(backend)
    dspy_version = getattr(dspy, "__version__", "")

    # Single template mode
    if RequestKey.TEMPLATE_STR in request:
        result = optimize_single_template(
            request[RequestKey.TEMPLATE_STR],
            training_data,
            cache_dir,
            optimizer_name,
            auto,
            metric_name,
            model,
            backend_fingerprint,
            dspy_version,
            no_cache,
        )
        return {
            ResponseKey.STATUS: Status.OK,
            ResponseKey.OPTIMIZED_TEMPLATE: result[ResponseKey.OPTIMIZED_TEMPLATE],
            ResponseKey.OPTIMIZED_INSTRUCTIONS: result[
                ResponseKey.OPTIMIZED_INSTRUCTIONS
            ],
            ResponseKey.OPTIMIZER: optimizer_name,
        }

    # Batch mode: multiple templates
    templates = request.get(RequestKey.TEMPLATES, [])
    results = []
    for tmpl in templates:
        tmpl_training = tmpl.get(RequestKey.TRAINING_DATA, training_data)
        result = optimize_single_template(
            tmpl[RequestKey.TEMPLATE_STR],
            tmpl_training,
            cache_dir,
            optimizer_name,
            auto,
            metric_name,
            model,
            backend_fingerprint,
            dspy_version,
            no_cache,
        )
        results.append(result)

    return {
        ResponseKey.STATUS: Status.OK,
        ResponseKey.RESULTS: results,
        ResponseKey.OPTIMIZER: optimizer_name,
        ResponseKey.COUNT: len(results),
    }
