/**
 * Validates documents against the `workflow-draft.v1` JSON Schema (
 * contracts, `workspace/contracts/schemas/workflow-draft.v1.json`). The
 * schema is vendored into `src/generated/workflow-draft-schema.ts` by
 * `npm run codegen`; this module wires it up to `ajv` and layers the two
 * semantic checks a JSON Schema can't express on its own: every
 * capability-bearing node's `config.capability` must be a member of the
 * draft's `capability_grants`, and the edge graph must be acyclic. Keep these
 * rules aligned with `workspace/contracts/tools/validate_contracts.py`
 * (`workflow_draft_errors`) and
 * `workspace/studio/crates/studio/src/workflow_draft.rs`
 * (`WorkflowDraft::validate`).
 */
// The workflow-draft.v1 schema declares `$schema:
// https://json-schema.org/draft/2020-12/schema`; ajv's default export only
// knows draft-07, so use the 2020-12 build.
import { default as Ajv2020, type ErrorObject } from "ajv/dist/2020.js";
import { WORKFLOW_DRAFT_V1_SCHEMA } from "./generated/workflow-draft-schema.js";

export interface WorkflowDraftValidationResult {
  readonly valid: boolean;
  readonly errors?: readonly ErrorObject[];
}

const ajv = new Ajv2020({ allErrors: true, strict: false });
const validateFn = ajv.compile(WORKFLOW_DRAFT_V1_SCHEMA);

/** Node kinds whose `config.capability` must be declared in the draft's
 * `capability_grants`. */
const CAPABILITY_NODE_KINDS = new Set(["tool", "memory_write", "memory_read"]);

interface DraftNodeLike {
  readonly id?: unknown;
  readonly kind?: unknown;
  readonly config?: unknown;
}

interface DraftEdgeLike {
  readonly source?: unknown;
  readonly target?: unknown;
}

function semanticError(message: string): ErrorObject {
  return {
    keyword: "semantic",
    instancePath: "",
    schemaPath: "",
    params: {},
    message,
  };
}

/** DFS cycle detection over the directed `source -> target` edge graph. */
function hasCycle(edges: readonly DraftEdgeLike[]): boolean {
  const adjacency = new Map<string, string[]>();
  for (const edge of edges) {
    if (typeof edge.source !== "string" || typeof edge.target !== "string") {
      continue;
    }
    const targets = adjacency.get(edge.source) ?? [];
    targets.push(edge.target);
    adjacency.set(edge.source, targets);
  }

  const WHITE = 0;
  const GREY = 1;
  const BLACK = 2;
  const color = new Map<string, number>();

  function visit(node: string): boolean {
    color.set(node, GREY);
    for (const neighbor of adjacency.get(node) ?? []) {
      const state = color.get(neighbor) ?? WHITE;
      if (state === GREY) {
        return true;
      }
      if (state === WHITE && visit(neighbor)) {
        return true;
      }
    }
    color.set(node, BLACK);
    return false;
  }

  for (const node of adjacency.keys()) {
    if ((color.get(node) ?? WHITE) === WHITE && visit(node)) {
      return true;
    }
  }
  return false;
}

/** The semantic checks layered on top of ajv's structural validation:
 * ungranted capabilities and edge cycles. Only operates on fields whose
 * shapes it confirms itself (arrays/objects/strings), so it is safe to call
 * even on structurally-invalid documents. */
function semanticErrors(doc: unknown): ErrorObject[] {
  if (typeof doc !== "object" || doc === null) {
    return [];
  }
  const draft = doc as { nodes?: unknown; edges?: unknown; capability_grants?: unknown };
  const errors: ErrorObject[] = [];

  const nodes = Array.isArray(draft.nodes) ? (draft.nodes as DraftNodeLike[]) : [];
  const granted = new Set(
    Array.isArray(draft.capability_grants)
      ? draft.capability_grants.filter((g): g is string => typeof g === "string")
      : [],
  );
  for (const node of nodes) {
    if (typeof node.kind !== "string" || !CAPABILITY_NODE_KINDS.has(node.kind)) {
      continue;
    }
    const config = typeof node.config === "object" && node.config !== null ? node.config : {};
    const capability = (config as { capability?: unknown }).capability;
    if (typeof capability === "string" && capability && !granted.has(capability)) {
      errors.push(
        semanticError(
          `node '${String(node.id)}': capability '${capability}' is not in capability_grants`,
        ),
      );
    }
  }

  const edges = Array.isArray(draft.edges) ? (draft.edges as DraftEdgeLike[]) : [];
  if (hasCycle(edges)) {
    errors.push(semanticError("edges: cycle detected"));
  }

  return errors;
}

/** Validate `doc` against the `apxm.workflow-draft.v1` JSON Schema, plus the
 * ungranted-capability and edge-cycle semantic checks ajv cannot express. */
export function validateWorkflowDraft(doc: unknown): WorkflowDraftValidationResult {
  const structurallyValid = validateFn(doc);
  const structuralErrors = structurallyValid ? [] : (validateFn.errors ?? []);
  const semantic = semanticErrors(doc);

  const errors = [...structuralErrors, ...semantic];
  if (errors.length === 0) {
    return { valid: true };
  }
  return { valid: false, errors };
}
