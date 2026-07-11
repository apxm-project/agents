from __future__ import annotations

from dataclasses import dataclass

from apxm import ToolGroup


SEARCH_WEB_CAPABILITY = "search_web"


@dataclass(frozen=True, slots=True)
class ProviderConnection:
    """Opaque provider/Auth connection reference supplied by the host."""

    reference: str | None


def provider_capabilities(connection: ProviderConnection | None) -> tuple[str, ...]:
    if connection is None or not (connection.reference or "").strip():
        return ()
    return (SEARCH_WEB_CAPABILITY,)


def capability_groups(connection: ProviderConnection | None = None) -> list[str]:
    groups = ["discovery", ToolGroup.SKILLS.value, ToolGroup.AUTHORING.value]
    if provider_capabilities(connection):
        groups.append(ToolGroup.WEB.value)
    return groups
