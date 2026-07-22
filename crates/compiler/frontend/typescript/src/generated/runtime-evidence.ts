// AUTO-GENERATED from apxm.runtime-evidence.v1; DO NOT EDIT.
export const RUNTIME_FACT_KINDS = ["instance.state_changed", "invocation.state_changed", "instance.created", "invocation.admitted", "child.attached", "attempt.recorded", "invocation.committed", "invocation.failed", "invocation.cancelled", "event.created", "event.await_registered", "invocation.parked", "event.terminal", "invocation.resumed", "instance.closed", "instance.cancelled", "effect.outcome_unknown", "delivery.recorded", "region.occurrence_started", "node_execution.recorded", "hook.executed", "context.transitioned"] as const;
export const ALL_FACT_KINDS = [...RUNTIME_FACT_KINDS, "LoopIterationCompleted"] as const;
export type RuntimeFactKind = (typeof RUNTIME_FACT_KINDS)[number];

export type RuntimeFact = {
  readonly fact_id: string;
  readonly event_sequence: number;
  readonly fact_kind: RuntimeFactKind;
};
export type NodeExecutionScope =
  | { readonly scope_kind: "non_loop" }
  | { readonly scope_kind: "loop"; readonly region_occurrence_id: string; readonly static_region_id: string; readonly loop_memberships: ReadonlyArray<{ readonly static_loop_id: string; readonly loop_occurrence_id: string }> };
export type NodeExecutionRecordedFact = {
  readonly fact_id: string;
  readonly event_sequence: number;
  readonly fact_kind: "node_execution.recorded";
  readonly node_execution_id: string;
  readonly air_node_id: string;
  readonly parent_node_execution_id?: string;
  readonly execution_scope: NodeExecutionScope;
};
export type LoopIterationCompletedFact = {
  readonly fact_id: string;
  readonly event_sequence: number;
  readonly fact_kind: "LoopIterationCompleted";
  readonly static_loop_id: string;
  readonly loop_occurrence_id: string;
  readonly iteration_index: number;
  readonly program_invocation_id: string;
  readonly causal_node_execution_ids: readonly [string, ...string[]];
};
export type Fact = RuntimeFact | NodeExecutionRecordedFact | LoopIterationCompletedFact;

const knownKinds = new Set<string>(ALL_FACT_KINDS);
function exact(value: Record<string, unknown>, required: readonly string[], optional: readonly string[] = []): void {
  const allowed = new Set([...required, ...optional]);
  for (const key of required) if (!(key in value)) throw new Error(`missing fact field: ${key}`);
  for (const key of Object.keys(value)) if (!allowed.has(key)) throw new Error(`unknown fact field: ${key}`);
}
export function decodeFact(input: unknown): Fact {
  if (input === null || typeof input !== "object" || Array.isArray(input)) throw new Error("fact must be an object");
  const value = input as Record<string, unknown>;
  const kind = value.fact_kind;
  if (typeof kind !== "string" || !knownKinds.has(kind)) throw new Error(`unknown fact_kind: ${String(kind)}`);
  if (kind === "LoopIterationCompleted") {
    exact(value, ["fact_id", "event_sequence", "fact_kind", "static_loop_id", "loop_occurrence_id", "iteration_index", "program_invocation_id", "causal_node_execution_ids"]);
    if (!Array.isArray(value.causal_node_execution_ids) || value.causal_node_execution_ids.length === 0) throw new Error("LoopIterationCompleted causality must be non-empty");
  } else if (kind === "node_execution.recorded") {
    exact(value, ["fact_id", "event_sequence", "fact_kind", "node_execution_id", "air_node_id", "execution_scope"], ["parent_node_execution_id"]);
    const scope = value.execution_scope as Record<string, unknown>;
    if (scope.scope_kind === "non_loop") exact(scope, ["scope_kind"]);
    else if (scope.scope_kind === "loop") {
      exact(scope, ["scope_kind", "region_occurrence_id", "static_region_id", "loop_memberships"]);
      if (!Array.isArray(scope.loop_memberships) || scope.loop_memberships.length === 0) throw new Error("loop scope memberships must be non-empty");
    } else throw new Error("unknown NodeExecution scope_kind");
  } else exact(value, ["fact_id", "event_sequence", "fact_kind"], ["ownership_epoch", "instance_state", "invocation_state", "event_state", "model_outcome", "commit_sequence", "node_execution_id", "attempt_id", "region_occurrence_id", "static_region_id", "loop_memberships", "air_node_id", "parent_node_execution_id", "hook_execution_id", "hook_id", "hook_scope", "hook_phase", "context_transition_id", "context_before_ref", "context_after_ref", "effect_outcome_ref", "typed_error"]);
  return value as Fact;
}
