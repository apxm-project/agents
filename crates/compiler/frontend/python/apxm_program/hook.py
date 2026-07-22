"""Static Hook bindings and the portable Agent Facade typing surface."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Generic, Protocol, TypeVar

from .agent_program import HookBinding

C = TypeVar("C")
T = TypeVar("T")


class AgentFacade(Protocol, Generic[C]):
    """Portable whole-program view passed to Hook callbacks at authoring time.

    Runtime execution supplies the live facade; the authoring frontend uses this
    protocol only to type-check Hook handler signatures.
    """

    @property
    def context(self) -> C: ...

    @context.setter
    def context(self, value: C) -> None: ...


@dataclass(frozen=True, slots=True)
class Hook:
    """Static Hook binding helper — lowers to one FrontendGraph hook_bindings row."""

    hook_id: str
    scope: str
    phase: str
    target_selector: str
    declaration_order: int
    handler_ref: str
    handler_digest: str
    input_type_ref: str
    output_type_ref: str
    return_mode: str = "observe"

    def to_binding(self) -> HookBinding:
        return HookBinding(
            hook_id=self.hook_id,
            scope=self.scope,
            phase=self.phase,
            target_selector=self.target_selector,
            declaration_order=self.declaration_order,
            handler_ref=self.handler_ref,
            handler_digest=self.handler_digest,
            input_type_ref=self.input_type_ref,
            output_type_ref=self.output_type_ref,
            return_mode=self.return_mode,
        )

    @classmethod
    def before_loop(
        cls,
        *,
        hook_id: str,
        target_selector: str,
        handler_ref: str,
        handler_digest: str,
        context_type_ref: str,
        declaration_order: int = 0,
    ) -> "Hook":
        return cls(
            hook_id=hook_id,
            scope="loop",
            phase="before",
            target_selector=target_selector,
            declaration_order=declaration_order,
            handler_ref=handler_ref,
            handler_digest=handler_digest,
            input_type_ref=context_type_ref,
            output_type_ref=context_type_ref,
            return_mode="observe",
        )

    @classmethod
    def after_model(
        cls,
        *,
        hook_id: str,
        target_selector: str,
        handler_ref: str,
        handler_digest: str,
        result_type_ref: str,
        declaration_order: int = 0,
        return_mode: str = "replace_result",
    ) -> "Hook":
        return cls(
            hook_id=hook_id,
            scope="model",
            phase="after",
            target_selector=target_selector,
            declaration_order=declaration_order,
            handler_ref=handler_ref,
            handler_digest=handler_digest,
            input_type_ref=result_type_ref,
            output_type_ref=result_type_ref,
            return_mode=return_mode,
        )
