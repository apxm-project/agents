# Workflow reviewer

Gao-local skill for reviewing staged or proposed APXM workflows for capability
coverage, trigger clarity, and grant requirements.

Use when the operator asks to review, audit, or sanity-check a workflow draft.

A "workflow draft" here means the `apxm.workflow-draft.v1` document (WF-1) —
the same contract `workflow-designer` authors against and the schema
`auditWorkflowDraft` structurally validates every Gao draft with (G-5:
required fields, the closed node-kind/edge-kind enums, acyclic graph). Review
against that contract, not an ad-hoc notion of "workflow shape" — see
`workflow-designer/STUDIO_CANVAS.md` for the node-kind/edge-kind vocabulary.
