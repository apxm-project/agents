// Immutable typed source tree for authored Agent programs.
//
// Mirrors `_bound_tree.py`: the frontend's semantic representation between
// the parsed AST and the language-neutral FrontendGraph. It retains lexical
// constructs, resolved declarations, inferred type references, and source
// spans. It carries no AIR or AIS operation name, no runtime value, and no
// mutable recorder state.
//
// `capture.ts` builds this tree and `emit.ts` folds it, exactly as
// `_capture.py` and `_emit.py` do, which is what makes the two frontends
// structurally the same below the AST walk rather than only reviewed by
// inspection.
//
// Where `_bound_tree.py` documents a closed set in a trailing comment, this
// file binds the generated vocabulary type instead — TypeScript can state it
// and Python cannot until the capture threads its records (design note, §4
// item 2), so the difference is one of what each language can express, not of
// what either tree holds.

import type {
  DeclKind,
  HookPhase,
  HookReturnMode,
  HookScope,
  InputContract,
  ParameterRole,
  PredicateComparator,
  RegionRole,
  ValueOrigin,
} from "./generated/frontend-graph.js";
import type {
  CallIntent,
  ControlIntent,
  PredicateLiteral,
  SkillRequirement,
  ValueExpression,
} from "./generated/frontend-records.js";
import type { Permission } from "./generated/permissions.js";

/** A source location range for one construct. */
export type Span = {
  readonly source_file: string;
  readonly start_line: number;
  readonly start_column: number;
  readonly end_line: number;
  readonly end_column: number;
};

/** One typed callback parameter with its resolved role. */
export type BoundParameter = {
  readonly value_id: string;
  readonly type_ref: string;
  readonly role: ParameterRole;
};

/** A resolved Context, Model, Tool, Capability, or Event declaration. */
export type BoundDeclaration = {
  readonly decl_id: string;
  readonly decl_kind: DeclKind;
  readonly input_type_ref: string;
  readonly output_type_ref: string;
  readonly target_ref?: string;
  readonly context_default_present?: boolean;
};

/** One typed value with exactly one origin. */
export type BoundValue = {
  readonly value_id: string;
  readonly type_ref: string;
  readonly origin: ValueOrigin;
  readonly origin_id?: string;
  readonly expression?: ValueExpression;
};

/** A typed use of a value at a named consumer slot. */
export type BoundOperand = {
  readonly value_id: string;
  readonly slot: string;
};

/**
 * A closed structural predicate over one prior typed value.
 *
 * The comparator is carried dynamically rather than as one of the contract's
 * three `ControlPredicate` branches, mirroring `_bound_tree.BoundPredicate`:
 * capture decides the branch from the authored comparison operator, and the
 * generated union serializer dispatches on the same discriminant when the
 * predicate reaches the graph.
 */
export type BoundPredicate = {
  readonly root_value_id: string;
  readonly property_path: readonly string[];
  readonly comparator: PredicateComparator;
  readonly literal?: PredicateLiteral;
};

/**
 * A resolved effectful call: model, tool, capability, agent, or event.
 *
 * Carries a generated `CallIntent` contract record plus the authoring-time
 * extras the contract does not state: a source `span` and typed `operands`
 * with consumer-slot identity (the contract instead carries `operand_values`
 * and leaves slot identity to `data_edges`). Mirrors `_bound_tree.BoundCall`.
 */
export type BoundCall = {
  readonly contract: CallIntent;
  readonly span?: Span;
  readonly operands: readonly BoundOperand[];
};

/**
 * A resolved structural construct: branch, loop, task group, try, yield, return.
 *
 * Carries a generated `ControlIntent` contract record plus the
 * authoring-time extras the contract does not state: a source `span` and
 * typed `operands` with consumer-slot identity. `predicate` is kept as its
 * own field rather than populated on `contract`, mirroring
 * `_bound_tree.BoundControl`; `emit.ts` puts it back on the contract record
 * when it folds the tree.
 */
export type BoundControl = {
  readonly contract: ControlIntent;
  readonly span?: Span;
  readonly operands: readonly BoundOperand[];
  readonly predicate?: BoundPredicate;
};

/** One lexical region owning ordered children. */
export type BoundRegion = {
  readonly region_id: string;
  readonly region_role: RegionRole;
  readonly parent_region_id?: string;
  readonly execution_order: number;
};

/** A static before/after Hook binding and its captured body region. */
export type BoundHook = {
  readonly hook_id: string;
  readonly scope: HookScope;
  readonly phase: HookPhase;
  readonly target_selector: string;
  readonly declaration_order: number;
  readonly handler_ref: string;
  readonly handler_digest: string;
  readonly input_type_ref: string;
  readonly output_type_ref: string;
  readonly return_mode: HookReturnMode;
  readonly body_region_id: string;
  readonly assigned_context_value_id?: string;
};

/** An explicit typed Context transition between two nodes. */
export type BoundContextEdge = {
  readonly from_node: string;
  readonly to_node: string;
  readonly context_type_ref: string;
  readonly value_id: string;
};

/**
 * One authored Capability declaration.
 *
 * Declarations are held one per authored binding, never one per
 * `capability_ref`: the same capability declared as both a Tool and a plain
 * Capability is two distinct requirements and both reach the FrontendGraph.
 */
export type BoundCapabilityRequirement = {
  readonly capability_ref: string;
  readonly tool_schema_present: boolean;
  readonly requested_permission?: Permission;
};

/** The immutable bound tree for one authored Agent program. */
export type BoundProgram = {
  readonly program_id: string;
  readonly entrypoint: string;
  readonly input_type_ref: string;
  readonly input_contract?: InputContract;
  readonly output_type_ref: string;
  readonly has_default_context: boolean;
  readonly context_type_ref?: string;
  readonly parameters: readonly BoundParameter[];
  readonly body_region_id: string;
  readonly declarations: readonly BoundDeclaration[];
  readonly values: readonly BoundValue[];
  readonly regions: readonly BoundRegion[];
  readonly calls: readonly BoundCall[];
  readonly controls: readonly BoundControl[];
  readonly context_edges: readonly BoundContextEdge[];
  readonly hooks: readonly BoundHook[];
  /** [program_ref, artifact_digest, entrypoint, target_agent_identity_requirement] */
  readonly imported_programs: readonly (readonly [string, string, string, string])[];
  readonly capability_requirements: readonly BoundCapabilityRequirement[];
  readonly model_requirements: readonly string[];
  /**
   * Authored Agent Skills, held as the generated contract record: unlike a
   * Capability requirement, nothing about a skill declaration is derived at
   * capture time, so there is nothing for a bound wrapper to add.
   */
  readonly skill_requirements: readonly SkillRequirement[];
  /** [node_id, span, semantic_annotation] */
  readonly spans: readonly (readonly [string, Span, string])[];
};

export type BoundNode = BoundCall | BoundControl;
