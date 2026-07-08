# Workflow designer

Gao-local skill for turning operator goals into structured APXM workflow plans
before any authoring write.

Use when the operator asks to create, draft, or design a workflow.

Studio "Apply workflow" drafts (`apxm_studio_workflow`) are a Gao-authored
instance of the canonical `apxm.workflow-draft.v1` contract — the same
schema Studio's own canvas persistence validates against and `auditWorkflowDraft`
checks Gao drafts with. This skill does not restate that schema by hand;
`STUDIO_CANVAS.md` teaches the parts of it Gao needs and points at the
schema-generated rules text Studio injects each turn
(`packages/widget-canvas/src/draft/workflow-draft-schema-rules.ts`) for the
closed node-kind/edge-kind enums and required fields.

See `prompt.md`, `STUDIO_CANVAS.md`, and `examples/` for operating recipes.
`examples/*.air` are vector fixtures
(`workspace/contracts/vectors/air/`, byte-identical copies) showing the AIR
shapes a workflow draft ultimately lowers to — not hand-written stand-ins.
