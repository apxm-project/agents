// Deterministic traversal from the bound tree to the FrontendGraph contract.
//
// One fold turns an immutable BoundProgram into the language-neutral
// `apxm.frontend-graph` value. Every reachable bound node produces exactly one
// graph record and every emitted value carries one typed origin. The fold reads
// the captured tree only; it never executes authored code.
//
// Per-record key layout is not written here: every `serialize*` call below
// projects one contract record through `generated/frontend-serializers.js`,
// which holds each record's required keys and then its stated optional ones in
// the contract's own order. What stays here is the fold itself plus the two
// derivations the contract does not state — `blocks()` and the loop
// `region_annotations`. Mirrors `_emit.py` function for function.

import type {
  BoundControl,
  BoundOperand,
  BoundProgram,
} from "./bound-tree.js";
import {
  FRONTEND_GRAPH_VERSION,
  SOURCE_MAP_VERSION,
  type Json,
} from "./contract.js";
import {
  CONTROL_KIND_LOOP,
  SOURCE_LANGUAGE_TYPESCRIPT,
  VALUE_ORIGIN_BLOCK_ARGUMENT,
  VALUE_ORIGIN_RESUME_INPUT,
  type SourceLanguage,
} from "./generated/frontend-graph.js";
import type { Block } from "./generated/frontend-records.js";
import {
  serializeBlock,
  serializeCallIntent,
  serializeCapabilityRequirement,
  serializeContextEdge,
  serializeControlIntent,
  serializeDataEdge,
  serializeDeclaration,
  serializeFunctionDef,
  serializeHookBinding,
  serializeImportedProgramRef,
  serializeModelRequirement,
  serializeProgramDefinition,
  serializeRegion,
  serializeValue,
} from "./generated/frontend-serializers.js";

/**
 * The one source-map annotation this frontend derives. It is a source-map
 * term, not a FrontendGraph vocabulary member, so no generated set states it.
 */
const STRUCTURAL_LOOP = "structural_loop";

/** Every region owns exactly one lexical entry block, named after the region. */
const ENTRY_BLOCK_SUFFIX = ".block.0";

/** Fold a bound program into the typed FrontendGraph contract value. */
export function emitFrontendGraph(
  program: BoundProgram,
  sourceLanguage: SourceLanguage = SOURCE_LANGUAGE_TYPESCRIPT,
): Json {
  // An intent carries its operand value ids; the slot each operand fills is
  // carried by the data edge instead, so both are emitted from one walk.
  const dataEdges: Json[] = [];

  const callIntents: Json[] = [];
  for (const call of program.calls) {
    callIntents.push(
      serializeCallIntent({
        ...call.contract,
        operand_values: operandValues(call.operands),
      }),
    );
    dataEdges.push(...dataEdgesFor(call.contract.node_id, call.operands));
  }

  const controlIntents: Json[] = [];
  for (const control of program.controls) {
    controlIntents.push(
      serializeControlIntent({
        ...control.contract,
        operand_values: operandValues(control.operands),
        predicate: control.predicate,
      }),
    );
    dataEdges.push(...dataEdgesFor(control.contract.node_id, control.operands));
  }

  const nodeSpans = program.spans.map(([nodeId, span, annotation]) => ({
    node_id: nodeId,
    source_file: span.source_file,
    span: {
      start_line: span.start_line,
      start_column: span.start_column,
      end_line: span.end_line,
      end_column: span.end_column,
    },
    semantic_annotation: annotation,
  }));
  // Frontend policy, not contract projection: a loop's first body region is
  // the one the compiler reads as the structural loop scope.
  const regionAnnotations = program.controls
    .filter(
      (control) =>
        control.contract.control_kind === CONTROL_KIND_LOOP &&
        (control.contract.body_region_ids?.length ?? 0) > 0,
    )
    .map((control) => ({
      region_id: (control.contract.body_region_ids as readonly string[])[0],
      annotation: STRUCTURAL_LOOP,
    }));

  return {
    schema_version: FRONTEND_GRAPH_VERSION,
    source_language: sourceLanguage,
    program_definitions: [
      serializeProgramDefinition({
        program_id: program.program_id,
        entrypoint: program.entrypoint,
        input_type_ref: program.input_type_ref,
        output_type_ref: program.output_type_ref,
        has_default_context: program.has_default_context,
        context_type_ref: program.context_type_ref,
      }),
    ],
    imported_program_refs: program.imported_programs.map(
      ([programRef, digest, entrypoint, identity]) =>
        serializeImportedProgramRef({
          program_ref: programRef,
          artifact_digest: digest,
          entrypoint,
          target_agent_identity_requirement: identity,
        }),
    ),
    declarations: program.declarations.map((declaration) =>
      serializeDeclaration(declaration),
    ),
    functions: [
      serializeFunctionDef({
        function_id: program.entrypoint,
        parameters: program.parameters,
        body_region_id: program.body_region_id,
        is_entrypoint: true,
        result_type_ref: program.output_type_ref,
      }),
    ],
    values: program.values.map((value) => serializeValue(value)),
    blocks: blocks(program).map((block) => serializeBlock(block)),
    regions: program.regions.map((region) => serializeRegion(region)),
    data_edges: dataEdges,
    call_intents: callIntents,
    control_intents: controlIntents,
    context_flow: program.context_edges.map((edge) => serializeContextEdge(edge)),
    hook_bindings: program.hooks.map((hook) => serializeHookBinding(hook)),
    capability_requirements: program.capability_requirements.map((requirement) =>
      serializeCapabilityRequirement(requirement),
    ),
    model_requirements: program.model_requirements.map((targetRef) =>
      serializeModelRequirement({ model_target_ref: targetRef }),
    ),
    source_map: {
      schema_version: SOURCE_MAP_VERSION,
      source_language: sourceLanguage,
      node_spans: nodeSpans,
      region_annotations: regionAnnotations,
    },
  };
}

/** The operand value ids an intent carries, or `undefined` when it takes none. */
function operandValues(
  operands: readonly BoundOperand[],
): readonly string[] | undefined {
  return operands.length === 0
    ? undefined
    : operands.map((operand) => operand.value_id);
}

/** One edge per operand, carrying the consumer slot the intent leaves out. */
function dataEdgesFor(
  nodeId: string,
  operands: readonly BoundOperand[],
): Json[] {
  return operands.map((operand) =>
    serializeDataEdge({
      from_value: operand.value_id,
      to_consumer: nodeId,
      consumer_slot: operand.slot,
    }),
  );
}

/**
 * Emit one lexical entry block for every captured region.
 *
 * A yielded resume value is introduced at the lexical point where the yield
 * resumes. Keeping that value in its enclosing region block makes the
 * continuation boundary explicit to Rust lowering without changing the value's
 * `resume_input` provenance. This is frontend policy: the contract states the
 * block record, not which region a resume value belongs to.
 */
function blocks(program: BoundProgram): Block[] {
  const controls = new Map<string, BoundControl>(
    program.controls.map((control) => [control.contract.node_id, control]),
  );
  const argumentsByRegion = new Map<string, string[]>(
    program.regions.map((region) => [region.region_id, []]),
  );
  for (const value of program.values) {
    if (value.origin === VALUE_ORIGIN_RESUME_INPUT && value.origin_id !== undefined) {
      const control = controls.get(value.origin_id);
      if (control !== undefined) {
        argumentsByRegion
          .get(control.contract.parent_region_id)
          ?.push(value.value_id);
      }
    } else if (
      value.origin === VALUE_ORIGIN_BLOCK_ARGUMENT &&
      value.origin_id !== undefined
    ) {
      const regionId = value.origin_id.endsWith(ENTRY_BLOCK_SUFFIX)
        ? value.origin_id.slice(0, -ENTRY_BLOCK_SUFFIX.length)
        : value.origin_id;
      argumentsByRegion.get(regionId)?.push(value.value_id);
    }
  }

  return program.regions.map((region) => ({
    block_id: `${region.region_id}${ENTRY_BLOCK_SUFFIX}`,
    region_id: region.region_id,
    block_arguments: argumentsByRegion.get(region.region_id) ?? [],
    execution_order: 0,
  }));
}
