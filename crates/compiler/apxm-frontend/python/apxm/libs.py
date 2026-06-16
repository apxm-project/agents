"""Agent-side import surface for the APXM skill library.

This module is the Python counterpart of ``import foo; foo.bar()`` for a
compiled APXM skill pack. ``load(skill_id)`` resolves a skill by manifest
id through the running ``apxm-server`` (the same loader the HTTP and MCP
surfaces use) and returns a :class:`SkillHandle`. ``SkillHandle.invoke``
posts to ``/v1/skills/{id}/execute`` and returns a :class:`SkillResult`
that mirrors the server's ``ExecuteResponse``.

Server reuse
============

``apxm.libs`` deliberately reuses the module-level ``httpx`` client and
server-URL resolution from :mod:`apxm.execution` — there is exactly one
HTTP transport for both ``/v1/execute`` (raw AIR) and
``/v1/skills/{id}/execute`` (named skill).

Example
-------

    from apxm.libs import load

    h = load("prompt-as-workflow")        # @latest
    h_pinned = load("apxm-orient@0.2.0")  # pinned version

    result = h.invoke(task="...")
    print(result.content)
    print(result.results)
"""

from __future__ import annotations

import asyncio
import threading
from dataclasses import dataclass, field
from typing import Any

from . import execution as _execution
from .errors import ServerError

# Route paths stay centralized so Python skill calls do not drift from the
# server-owned HTTP surface. The Python client mirrors the path shape but
# resolves the skill id at call time.
_SKILL_DETAIL_PATH = "/v1/skills/{id}"
_SKILL_EXECUTE_PATH = "/v1/skills/{id}/execute"


@dataclass(slots=True)
class SkillResult:
    """Result of a :meth:`SkillHandle.invoke` call.

    Mirrors the server's ``ExecuteResponse`` plus the wrapping
    ``execution_id`` returned by ``/v1/skills/{id}/execute``. ``content``
    is the first string output if present, ``results`` is the full
    ``{token: value}`` map (including any ``node_output_map`` style
    keys), and ``session_dir`` points at the persisted session bundle
    when the server emitted one.
    """

    execution_id: str | None = None
    content: str | None = None
    session_dir: str | None = None
    results: dict[str, Any] = field(default_factory=dict)
    stats: dict[str, Any] = field(default_factory=dict)
    llm_usage: dict[str, Any] = field(default_factory=dict)

    @classmethod
    def from_response(cls, data: dict[str, Any]) -> "SkillResult":
        if not isinstance(data, dict):
            raise TypeError("SkillResult response must be a JSON object")
        return cls(
            execution_id=data.get("execution_id"),
            content=data.get("content"),
            session_dir=data.get("session_dir"),
            results=data.get("results", {}) or {},
            stats=data.get("stats", {}) or {},
            llm_usage=data.get("llm_usage", {}) or {},
        )


@dataclass(slots=True)
class SkillHandle:
    """Handle to a resolved APXM skill.

    Returned by :func:`load`. ``invoke()`` (or ``invoke_async``) posts to
    the running ``apxm-server`` and returns a :class:`SkillResult`. The
    handle caches the skill's declared input order so ``invoke(**kwargs)``
    can translate keyword arguments to the positional ``args: Vec<String>``
    the server expects.
    """

    skill_id: str
    version: str | None = None
    input_order: tuple[str, ...] = ()

    def invoke(
        self,
        *,
        session_id: str | None = None,
        **kwargs: Any,
    ) -> SkillResult:
        """Execute the skill synchronously and return a :class:`SkillResult`.

        Keyword arguments are matched against the skill's declared
        ``inputs`` (resolved at :func:`load` time) and serialized to
        strings in declared order. Unknown kwargs raise ``TypeError``.
        Missing required kwargs raise ``TypeError`` too.
        """
        return _run_sync(self.invoke_async(session_id=session_id, **kwargs))

    async def invoke_async(
        self,
        *,
        session_id: str | None = None,
        **kwargs: Any,
    ) -> SkillResult:
        """Async counterpart of :meth:`invoke`."""
        args = _kwargs_to_args(self.skill_id, self.input_order, kwargs)
        body: dict[str, Any] = {"args": args}
        if session_id is not None:
            body["session_id"] = session_id
        client = await _execution._get_client()
        response = await client.post(
            _SKILL_EXECUTE_PATH.format(id=self.skill_id), json=body
        )
        if response.status_code != 200:
            raise ServerError(
                f"/v1/skills/{self.skill_id}/execute returned "
                f"{response.status_code}: {response.text}"
            )
        return SkillResult.from_response(response.json())


def load(skill_id: str) -> SkillHandle:
    """Resolve a skill by manifest id and return a :class:`SkillHandle`.

    Accepts ``<skill_id>`` (latest version) or ``<skill_id>@<version>``;
    both formats are forwarded to the server's
    :func:`apxm_server::SkillLibrary::find_executable`, which is the
    single source of truth for skill resolution (same path used by
    ``/v1/skills/{id}/execute`` and the MCP ``skill_call`` tool).

    A successful load fetches the skill's declared input order so that
    later :meth:`SkillHandle.invoke` calls can map ``**kwargs`` to
    positional ``args``. The server contact is one ``GET
    /v1/skills/{id}`` call; the running server is required.
    """
    if not isinstance(skill_id, str) or not skill_id.strip():
        raise TypeError("load() requires a non-empty skill id string")
    return _run_sync(_load_async(skill_id))


async def _load_async(skill_id: str) -> SkillHandle:
    client = await _execution._get_client()
    response = await client.get(_SKILL_DETAIL_PATH.format(id=skill_id))
    if response.status_code == 404:
        raise ServerError(f"skill not found: {skill_id}")
    if response.status_code != 200:
        raise ServerError(
            f"/v1/skills/{skill_id} returned "
            f"{response.status_code}: {response.text}"
        )
    payload = response.json()
    manifest = payload.get("manifest") or {}
    inputs = manifest.get("inputs") or []
    input_order: list[str] = []
    for entry in inputs:
        if isinstance(entry, dict):
            name = entry.get("name")
            if isinstance(name, str):
                input_order.append(name)
    return SkillHandle(
        skill_id=manifest.get("skill_id") or skill_id.split("@", 1)[0],
        version=manifest.get("version"),
        input_order=tuple(input_order),
    )


def _kwargs_to_args(
    skill_id: str,
    input_order: tuple[str, ...],
    kwargs: dict[str, Any],
) -> list[str]:
    """Translate ``**kwargs`` to the positional ``args: Vec<String>`` the
    server's ``SkillExecuteRequest`` expects. Order matches the skill's
    declared ``inputs``. Unknown kwargs raise ``TypeError`` — the skill
    declaration is the schema, and silent drops mask author errors (see
    `feedback_attribute_dual_naming` for the analogous bug).
    """
    if not input_order:
        if kwargs:
            raise TypeError(
                f"skill {skill_id!r} declares no inputs; "
                f"unexpected keyword arguments: {sorted(kwargs)}"
            )
        return []
    unknown = sorted(set(kwargs) - set(input_order))
    if unknown:
        raise TypeError(
            f"skill {skill_id!r} got unexpected keyword arguments: {unknown}; "
            f"declared inputs: {list(input_order)}"
        )
    args: list[str] = []
    for name in input_order:
        if name not in kwargs:
            # Missing inputs are forwarded as empty strings: the server's
            # runtime handler is the single authority on which inputs are
            # actually required. Passing nothing positionally would
            # silently shift later args.
            args.append("")
            continue
        value = kwargs[name]
        if isinstance(value, str):
            args.append(value)
        else:
            args.append(str(value))
    return args


def _run_sync(coro: Any) -> Any:
    """Run an awaitable synchronously even when a running event loop is
    present (e.g. inside Jupyter). Mirrors
    :meth:`apxm.execution.CompiledFlow.run_sync` so library callers do
    not need to know whether the underlying transport is async.
    """
    result: list[Any] = []
    error: list[BaseException] = []

    def _runner() -> None:
        try:
            result.append(asyncio.run(coro))
        except BaseException as exc:  # noqa: BLE001 - re-raised below
            error.append(exc)

    thread = threading.Thread(target=_runner)
    thread.start()
    thread.join()
    if error:
        raise error[0]
    return result[0]


__all__ = ["SkillHandle", "SkillResult", "load"]
