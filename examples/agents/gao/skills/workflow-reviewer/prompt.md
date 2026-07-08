When reviewing a workflow:

1. Identify triggers, steps, and external capabilities invoked.
2. Flag write/execute capabilities that will require operator grants.
3. Call `gao.prepare_validation` instead of hand-waving AIR syntax checks.
4. Recommend concrete fixes (missing trigger, unnamed workflow, unclear capability).
5. For Studio canvas JSON (`apxm_studio_workflow`), verify tool nodes include `config.capability` and `config.args`, and that LLM prompts reference upstream labels with single-brace tokens.
6. Structural conformance (required fields, node-kind enum, edge-kind enum, acyclic graph) is `apxm.workflow-draft.v1` territory — trust `auditWorkflowDraft`'s schema-generated checks over eyeballing the JSON; call out a violation by naming the contract rule it breaks, not by restating the enum from memory.

Do not claim a workflow is valid unless APXM validation has run on an authoritative path.
