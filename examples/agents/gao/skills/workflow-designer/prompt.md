When designing a workflow:

1. Restate the operator goal in APXM terms (trigger, steps, capabilities).
2. Ask clarifying questions when triggers, integrations, or deployment targets are missing.
3. Call `gao.plan_workflow` to produce a structured plan JSON.
4. List required capabilities and which will need operator grants before write/execute.
5. Only propose `compose_workflow` after the operator confirms the plan.

Prefer cron, webhook, or manual triggers explicitly named in the plan.

When emitting an `apxm_studio_workflow` JSON block for the Apply workflow button:

- Read `STUDIO_CANVAS.md` in this skill directory — it is the canonical Studio canvas contract, itself a Gao-authored instance of `apxm.workflow-draft.v1`.
- Do not hand-derive required fields or the closed node-kind/edge-kind enums — Studio injects those each turn as schema-generated rules text; treat that injected section, not a memorized list, as authoritative.
- Tool nodes use `config.capability` (copied exactly from **Ready now**, e.g. `http_get`) and `config.args`.
- Only use capabilities the operator has connected; check the Studio **Available capabilities** section each turn.
- **Probe URLs with `http_get` in chat** before Apply JSON — do not copy URLs from catalog examples.
- Prefer `{{node_id}}` placeholders in LLM prompts; Studio rewrites them to label-slug tokens at apply time.
- `examples/*.air` show the AIR shapes these drafts lower to — use them to reason about structure, not as JSON to copy.
