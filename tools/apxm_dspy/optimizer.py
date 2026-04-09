"""DSPy optimizer — configure LM, run optimizer, extract results."""

from __future__ import annotations

import os

import dspy

from .cache import compute_cache_key, hash_training_data, load_cached, store_cached
from .metrics import build_metric
from .signatures import extract_fields_from_template
from .types import (
    DEFAULT_DSPY_PREFIX,
    AutoLevel,
    Metric,
    Optimizer,
    Protocol,
    Status,
    PROTOCOL_TO_DSPY_PREFIX,
)


def _resolve_env(value: str, default: str = "") -> str:
    """Resolve 'env:VAR_NAME' to its environment variable value."""
    if isinstance(value, str) and value.startswith("env:"):
        return os.environ.get(value[4:], default)
    return value


def configure_lm(backend: dict) -> dspy.LM:
    """Map APXM backend config to dspy.LM()."""
    protocol = backend.get("protocol", Protocol.OPENAI)
    try:
        proto = Protocol(protocol)
        prefix = PROTOCOL_TO_DSPY_PREFIX.get(proto, DEFAULT_DSPY_PREFIX)
    except ValueError:
        prefix = DEFAULT_DSPY_PREFIX
    model = backend.get("model", "gpt-4o-mini")

    api_key = _resolve_env(backend.get("api_key", ""))
    endpoint = backend.get("endpoint")
    resolved_headers = {
        k: _resolve_env(v, v) for k, v in backend.get("headers", {}).items()
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
    optimizer_name: str,
    auto: str,
    metric_name: str,
    model: str = "",
) -> dict:
    """Optimize a single template using DSPy."""
    # Check cache first
    td_hash = hash_training_data(training_data)
    cache_key = compute_cache_key(template_str, td_hash, optimizer_name, model)
    cached = load_cached(cache_key)
    if cached is not None:
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

    # Reconstruct template preserving {N} placeholders
    optimized_template = optimized_instruction + "\n\n" + template_str

    result = {
        "optimized_template": optimized_template,
        "optimized_instructions": optimized_instruction,
        "original_template": template_str,
    }
    store_cached(cache_key, result)
    return result


def optimize_templates(request: dict) -> dict:
    """Process a batch request to optimize one or more templates."""
    backend = request.get("backend", {})
    lm = configure_lm(backend)
    dspy.configure(lm=lm)

    training_data = request.get("training_data", [])
    optimizer_name = request.get("optimizer", Optimizer.MIPRO)
    auto = request.get("auto", AutoLevel.LIGHT)
    metric_name = request.get("metric", Metric.TOKEN_OVERLAP)
    model = backend.get("model", "")

    # Single template mode
    if "template_str" in request:
        result = optimize_single_template(
            request["template_str"],
            training_data,
            optimizer_name,
            auto,
            metric_name,
            model,
        )
        return {
            "status": Status.OK,
            "optimized_template": result["optimized_template"],
            "optimized_instructions": result["optimized_instructions"],
            "optimizer": optimizer_name,
        }

    # Batch mode: multiple templates
    templates = request.get("templates", [])
    results = []
    for tmpl in templates:
        tmpl_training = tmpl.get("training_data", training_data)
        result = optimize_single_template(
            tmpl["template_str"],
            tmpl_training,
            optimizer_name,
            auto,
            metric_name,
            model,
        )
        results.append(result)

    return {
        "status": Status.OK,
        "results": results,
        "optimizer": optimizer_name,
        "count": len(results),
    }
