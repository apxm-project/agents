"""Typed constants for DSPy integration — mirrors apxm_core::constants::dspy."""

from __future__ import annotations

from enum import StrEnum


class Optimizer(StrEnum):
    """DSPy optimizer algorithms."""

    MIPRO = "mipro"
    BOOTSTRAP = "bootstrap"
    COPRO = "copro"


class AutoLevel(StrEnum):
    """DSPy MIPROv2 auto-tuning levels."""

    LIGHT = "light"
    MEDIUM = "medium"
    HEAVY = "heavy"


class Metric(StrEnum):
    """Built-in metric functions for DSPy optimization."""

    TOKEN_OVERLAP = "token_overlap"
    EXACT_MATCH = "exact_match"
    CONTAINS = "contains"
    LLM_JUDGE = "llm_judge"


class Protocol(StrEnum):
    """APXM backend protocols mapped to DSPy LM providers."""

    OPENAI = "openai"
    ANTHROPIC = "anthropic"
    OLLAMA = "ollama"
    VLLM = "vllm"
    GOOGLE = "google"


class Status(StrEnum):
    """Response status values."""

    OK = "ok"
    ERROR = "error"


# DSPy LM provider prefix for each protocol
PROTOCOL_TO_DSPY_PREFIX: dict[Protocol, str] = {
    Protocol.OPENAI: "openai",
    Protocol.ANTHROPIC: "anthropic",
    Protocol.OLLAMA: "ollama_chat",
    Protocol.VLLM: "openai",
    Protocol.GOOGLE: "google",
}

# Default DSPy prefix for unknown protocols
DEFAULT_DSPY_PREFIX = "openai"
