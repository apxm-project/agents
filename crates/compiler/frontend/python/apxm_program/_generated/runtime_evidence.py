# AUTO-GENERATED from apxm.runtime-evidence.v1; DO NOT EDIT.
from dataclasses import dataclass
from typing import Any, Final, TypeAlias

RUNTIME_FACT_KINDS: Final[frozenset[str]] = frozenset(("instance.state_changed", "invocation.state_changed", "instance.created", "invocation.admitted", "child.attached", "attempt.recorded", "invocation.committed", "invocation.failed", "invocation.cancelled", "event.created", "event.await_registered", "invocation.parked", "event.terminal", "invocation.resumed", "instance.closed", "instance.cancelled", "effect.outcome_unknown", "delivery.recorded", "region.occurrence_started", "node_execution.recorded", "hook.executed", "context.transitioned"))
LOOP_ITERATION_COMPLETED: Final[str] = "LoopIterationCompleted"
NODE_EXECUTION_RECORDED: Final[str] = "node_execution.recorded"
ALL_FACT_KINDS: Final[frozenset[str]] = RUNTIME_FACT_KINDS | frozenset((LOOP_ITERATION_COMPLETED,))

@dataclass(frozen=True)
class RuntimeFact:
    fact_id: str
    event_sequence: int
    fact_kind: str

@dataclass(frozen=True)
class NodeExecutionRecordedFact:
    fact_id: str
    event_sequence: int
    node_execution_id: str
    air_node_id: str
    execution_scope: dict[str, Any]
    parent_node_execution_id: str | None = None

@dataclass(frozen=True)
class LoopIterationCompletedFact:
    fact_id: str
    event_sequence: int
    static_loop_id: str
    loop_occurrence_id: str
    iteration_index: int
    program_invocation_id: str
    causal_node_execution_ids: tuple[str, ...]

Fact: TypeAlias = RuntimeFact | NodeExecutionRecordedFact | LoopIterationCompletedFact

def _exact(value: dict[str, Any], required: set[str], optional: set[str] = set()) -> None:
    missing = required - value.keys()
    unknown = value.keys() - required - optional
    if missing or unknown:
        raise ValueError(f"invalid fact fields missing={sorted(missing)} unknown={sorted(unknown)}")

def decode_fact(value: dict[str, Any]) -> Fact:
    if not isinstance(value, dict):
        raise ValueError("fact must be an object")
    kind = value.get("fact_kind")
    if kind not in ALL_FACT_KINDS:
        raise ValueError(f"unknown fact_kind: {kind!r}")
    if kind == LOOP_ITERATION_COMPLETED:
        required = {"fact_id", "event_sequence", "fact_kind", "static_loop_id", "loop_occurrence_id", "iteration_index", "program_invocation_id", "causal_node_execution_ids"}
        _exact(value, required)
        causal = tuple(value["causal_node_execution_ids"])
        if not causal:
            raise ValueError("LoopIterationCompleted causality must be non-empty")
        return LoopIterationCompletedFact(value["fact_id"], value["event_sequence"], value["static_loop_id"], value["loop_occurrence_id"], value["iteration_index"], value["program_invocation_id"], causal)
    if kind == NODE_EXECUTION_RECORDED:
        required = {"fact_id", "event_sequence", "fact_kind", "node_execution_id", "air_node_id", "execution_scope"}
        _exact(value, required, {"parent_node_execution_id"})
        scope = value["execution_scope"]
        if scope.get("scope_kind") == "non_loop":
            _exact(scope, {"scope_kind"})
        elif scope.get("scope_kind") == "loop":
            _exact(scope, {"scope_kind", "region_occurrence_id", "static_region_id", "loop_memberships"})
            if not scope["loop_memberships"]:
                raise ValueError("loop scope memberships must be non-empty")
        else:
            raise ValueError("unknown NodeExecution scope_kind")
        return NodeExecutionRecordedFact(value["fact_id"], value["event_sequence"], value["node_execution_id"], value["air_node_id"], scope, value.get("parent_node_execution_id"))
    runtime_optional = {"ownership_epoch", "instance_state", "invocation_state", "event_state", "model_outcome", "commit_sequence", "node_execution_id", "attempt_id", "region_occurrence_id", "static_region_id", "loop_memberships", "air_node_id", "parent_node_execution_id", "hook_execution_id", "hook_id", "hook_scope", "hook_phase", "context_transition_id", "context_before_ref", "context_after_ref", "effect_outcome_ref", "typed_error"}
    _exact(value, {"fact_id", "event_sequence", "fact_kind"}, runtime_optional)
    return RuntimeFact(value["fact_id"], value["event_sequence"], kind)
