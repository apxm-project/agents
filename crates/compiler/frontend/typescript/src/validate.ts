/**
 * Validates documents against the `workflow-draft.v1` JSON Schema (
 * contracts, `workspace/contracts/schemas/workflow-draft.v1.json`). The
 * schema is vendored into `src/generated/workflow-draft-schema.ts` by
 * `npm run codegen`; this module wires it up to `ajv`. Compiler semantics are
 * checked after canonical FrontendGraph lowering.
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

/** Validate `doc` against the `apxm.workflow-draft.v1` JSON Schema. */
export function validateWorkflowDraft(doc: unknown): WorkflowDraftValidationResult {
  const structurallyValid = validateFn(doc);
  if (structurallyValid) {
    return { valid: true };
  }
  return { valid: false, errors: validateFn.errors ?? [] };
}
