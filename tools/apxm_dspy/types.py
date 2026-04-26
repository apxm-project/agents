"""Typed constants for DSPy integration — mirrors apxm_core::constants::dspy."""

from __future__ import annotations

try:
    from enum import StrEnum
except ImportError:  # Python 3.10 in some Dekk-managed test environments.
    from enum import Enum

    class StrEnum(str, Enum):
        pass


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


class RequestKey(StrEnum):
    """Optimizer request keys shared with the compiler subprocess contract."""

    TRAINING_DATA = "training_data"
    TRAINING_DATA_PATH = "training_data_path"
    BACKEND = "backend"
    BACKEND_JSON = "backend_json"
    CACHE_DIR = "cache_dir"
    OPTIMIZER = "optimizer"
    AUTO = "auto"
    METRIC = "metric"
    MODEL = "model"
    TEMPLATE_STR = "template_str"
    TEMPLATES = "templates"
    NO_CACHE = "no_cache"


class ResponseKey(StrEnum):
    """Optimizer response keys shared with the compiler subprocess contract."""

    STATUS = "status"
    ERROR = "error"
    RESULTS = "results"
    OPTIMIZED_TEMPLATE = "optimized_template"
    OPTIMIZED_INSTRUCTIONS = "optimized_instructions"
    ORIGINAL_TEMPLATE = "original_template"
    ORIGINAL_TEMPLATE_CHARS = "original_template_chars"
    OPTIMIZED_TEMPLATE_CHARS = "optimized_template_chars"
    TEMPLATE_CHAR_DELTA = "template_char_delta"
    OPTIMIZED_INSTRUCTION_CHARS = "optimized_instruction_chars"
    TRAINING_EXAMPLES = "training_examples"
    CACHE_HIT = "cache_hit"
    OPTIMIZER = "optimizer"
    COUNT = "count"


class BackendKey(StrEnum):
    """Backend config keys consumed by the DSPy adapter."""

    PROTOCOL = "protocol"
    MODEL = "model"
    API_KEY = "api_key"
    ENDPOINT = "endpoint"
    HEADERS = "headers"


ENV_NO_CACHE = "APXM_NO_CACHE"
ENV_VALUE_PREFIX = "env:"
TRUE_ENV_VALUES = frozenset(("1", "true"))
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
