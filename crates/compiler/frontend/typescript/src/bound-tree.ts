// Immutable typed source tree for authored Agent programs.
//
// Mirrors `_bound_tree.py`: the frontend's semantic representation between
// the parsed AST and the language-neutral FrontendGraph. It retains lexical
// constructs, resolved declarations, inferred type references, and source
// spans. It carries no AIR or AIS operation name, no runtime value, and no
// mutable recorder state.
//
// This file is new (§4 item 3 of the frontend-vocabulary-generation design
// note): TypeScript has no bound tree today — `capture.ts` builds
// wire-shaped `Json` objects inline instead. Introducing this file makes the
// two frontends' parity structural (the same record shapes exist in both
// languages) rather than only reviewed by inspection. `capture.ts` is not
// wired to build or consume this tree yet; that fold-restructuring is out of
// this change's scope (see the design note's §5).

import type { Json } from "./contract.js";
import type { CallIntent, ControlIntent } from "./generated/frontend-records.js";
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
  /** agent_facade | input | context | ordinary */
  readonly role: string;
};

/** A resolved Context, Model, Tool, Capability, or Event declaration. */
export type BoundDeclaration = {
  readonly decl_id: string;
  /** context | model_binding | tool_binding | capability_binding | event_type */
  readonly decl_kind: string;
  readonly input_type_ref: string;
  readonly output_type_ref: string;
  readonly target_ref?: string;
  readonly context_default_present?: boolean;
};

/** One typed value with exactly one origin. */
export type BoundValue = {
  readonly value_id: string;
  readonly type_ref: string;
  /** parameter | call_result | block_argument | context_value | literal | resume_input */
  readonly origin: string;
  readonly origin_id?: string;
  readonly expression?: Json;
};

/** A typed use of a value at a named consumer slot. */
export type BoundOperand = {
  readonly value_id: string;
  readonly slot: string;
};

/** A closed structural predicate over one prior typed value. */
export type BoundPredicate = {
  readonly root_value_id: string;
  readonly property_path: readonly string[];
  /** truthy | equals | not_equals */
  readonly comparator: string;
  readonly literal?: Json;
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
 * own field rather than populated on `contract`: the generated
 * `ControlIntent.predicate` is typed as the generated `ControlPredicate`
 * union, whose `EqualsPredicate`/`NotEqualsPredicate` branches carry a
 * generated `PredicateLiteral`, and threading capture's predicate
 * construction through those generated types is out of this change's scope
 * (see the design note, §4 item 2). Mirrors `_bound_tree.BoundControl`.
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
  /** function_body | conditional_arm | loop_body | task_scope | task_child | try_body | catch_body */
  readonly region_role: string;
  readonly parent_region_id?: string;
  readonly execution_order: number;
};

/** A static before/after Hook binding. */
export type BoundHook = {
  readonly hook_id: string;
  readonly scope: string;
  readonly phase: string;
  readonly target_selector: string;
  readonly declaration_order: number;
  readonly handler_ref: string;
  readonly handler_digest: string;
  readonly input_type_ref: string;
  readonly output_type_ref: string;
  readonly return_mode: string;
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
  /** [node_id, span, semantic_annotation] */
  readonly spans: readonly (readonly [string, Span, string])[];
};

export type BoundNode = BoundCall | BoundControl;
