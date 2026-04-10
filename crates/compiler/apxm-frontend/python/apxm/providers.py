"""LLM provider registry — mirrors APXM's BUILTIN_PROVIDERS.

This module is the single source of truth for valid LLM providers on the
Python side.  Every provider listed here corresponds to a registered backend
in the APXM runtime (``apxm-backends`` crate).  If a provider is not in this
registry, APXM will reject it at construction time rather than letting
it fail silently at runtime.

To add a new provider:
  1. Add it to ``LlmProvider`` enum in ``apxm-core/src/defs.rs``
  2. Implement ``LLMBackend`` in ``apxm-backends``
  3. Add the entry below
"""

from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True, slots=True)
class ProviderSpec:
    """Metadata for a registered LLM provider."""

    name: str
    requires_api_key: bool
    api_key_env_var: str
    default_base_url: str | None = None
    aliases: tuple[str, ...] = ()


# ── Registry ────────────────────────────────────────────────────────────
# Must stay in sync with apxm-core/src/defs.rs PROVIDERS constant and
# apxm-backends/src/llm/provider.rs ProviderId enum.
# ─────────────────────────────────────────────────────────────────────────

REGISTERED_PROVIDERS: dict[str, ProviderSpec] = {}

_SPECS: list[ProviderSpec] = [
    ProviderSpec(
        name="ollama",
        requires_api_key=False,
        api_key_env_var="OLLAMA_API_KEY",
        default_base_url="http://localhost:11434",
    ),
    ProviderSpec(
        name="openai",
        requires_api_key=True,
        api_key_env_var="OPENAI_API_KEY",
    ),
    ProviderSpec(
        name="anthropic",
        requires_api_key=True,
        api_key_env_var="ANTHROPIC_API_KEY",
    ),
    ProviderSpec(
        name="google",
        requires_api_key=True,
        api_key_env_var="GOOGLE_API_KEY",
        aliases=("gemini",),
    ),
    ProviderSpec(
        name="openrouter",
        requires_api_key=True,
        api_key_env_var="OPENROUTER_API_KEY",
        default_base_url="https://openrouter.ai/api/v1",
    ),
]

# Build lookup table (canonical name + aliases)
for _spec in _SPECS:
    REGISTERED_PROVIDERS[_spec.name] = _spec
    for _alias in _spec.aliases:
        REGISTERED_PROVIDERS[_alias] = _spec


def resolve_provider(name: str) -> ProviderSpec:
    """Resolve a provider name (or alias) to its spec.

    Raises ``ValueError`` with a clear message if the provider is not
    registered with the APXM runtime.
    """
    spec = REGISTERED_PROVIDERS.get(name.lower())
    if spec is None:
        valid = sorted({s.name for s in _SPECS})
        raise ValueError(
            f"Unknown provider '{name}'. "
            f"Registered APXM providers: {', '.join(valid)}. "
            f"To add a custom provider, register it in apxm-core and apxm-backends first."
        )
    return spec


def list_providers() -> list[str]:
    """Return canonical names of all registered providers."""
    return sorted({s.name for s in _SPECS})
