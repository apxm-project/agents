import { readFileSync } from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";
import { validateWorkflowDraft } from "../src/validate.js";

const VECTOR_PATH = path.resolve(
  new URL(".", import.meta.url).pathname,
  "../../../../../../../workspace/contracts/vectors/workflow-draft.v1.json",
);

interface DraftVector {
  name: string;
  input: unknown;
  expected_valid: boolean;
  expected_reason?: string;
}

describe("validateWorkflowDraft", () => {
  it("accepts a hand-built minimal valid draft", () => {
    const result = validateWorkflowDraft({
      schema_version: "apxm.workflow-draft.v1",
      name: "echo-flow",
      nodes: [{ id: "ask-1", kind: "text", label: "Ask user" }],
      edges: [],
    });
    expect(result.valid).toBe(true);
    expect(result.errors).toBeUndefined();
  });

  it("rejects a draft missing required fields", () => {
    const result = validateWorkflowDraft({
      name: "bad",
      nodes: [],
      edges: [],
    });
    expect(result.valid).toBe(false);
    expect(result.errors?.length).toBeGreaterThan(0);
  });

  it("rejects a draft with wrong field types", () => {
    const result = validateWorkflowDraft({
      schema_version: "apxm.workflow-draft.v1",
      name: "bad-types",
      nodes: "not-an-array",
      edges: [],
    });
    expect(result.valid).toBe(false);
  });

  it("matches contracts vectors that are structurally invalid", () => {
    const raw = readFileSync(VECTOR_PATH, "utf-8");
    const vectors: DraftVector[] = JSON.parse(raw);
    expect(vectors.length).toBeGreaterThan(0);
    for (const vector of vectors.filter((item) => item.expected_reason === "unknown_node_kind")) {
      const result = validateWorkflowDraft(vector.input);
      expect(result.valid, `vector '${vector.name}'`).toBe(vector.expected_valid);
    }
  });

  it("leaves workflow semantic vectors to the compiler path", () => {
    const raw = readFileSync(VECTOR_PATH, "utf-8");
    const vectors: DraftVector[] = JSON.parse(raw);
    for (const vector of vectors.filter((item) =>
      item.expected_reason === "ungranted_capability" || item.expected_reason === "cycle"
    )) {
      const result = validateWorkflowDraft(vector.input);
      expect(result.valid, `vector '${vector.name}'`).toBe(true);
    }
  });
});
