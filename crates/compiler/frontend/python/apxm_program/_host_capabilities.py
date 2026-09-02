"""The host-fulfilled Capability ids the package being captured declares.

A host capability is declared in ``agent.toml`` as ``[[capabilities.host]]`` and
referred to as ``Capability("host:<id>")``. The runtime never executes one; the
embedding host answers the request (ADR-0025). The declarations live in the
package manifest, which the confined interpreter never reads, so the trusted
capture harness supplies them before the submitted module runs and the minted
Capability set becomes the builtin catalogue united with these.

This module is private. A program that could declare its own host capabilities
would be minting its own authority.
"""

from __future__ import annotations

from typing import Iterable

HOST_CAPABILITY_REF_PREFIX = "host:"

_declared: frozenset[str] = frozenset()


def set_declared_host_capabilities(ids: Iterable[str]) -> None:
    """Supply the host capability ids the package manifest declares."""
    global _declared
    _declared = frozenset(f"{HOST_CAPABILITY_REF_PREFIX}{identifier}" for identifier in ids)


def is_declared_host_capability(reference: str) -> bool:
    """Whether this package declares the host capability ``reference`` names."""
    return reference in _declared


def declared_host_capabilities() -> tuple[str, ...]:
    """The references this package mints, in a stable order, for a diagnostic."""
    return tuple(sorted(_declared))
