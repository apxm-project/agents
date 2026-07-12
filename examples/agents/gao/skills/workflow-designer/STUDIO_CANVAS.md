# Studio canvas JSON (`apxm_studio_workflow`)

Gao uses this shape when the operator should click **Apply workflow** in Chat.

This is a Gao-authored draft of the canonical `apxm.workflow-draft.v1` contract
(`workspace/contracts/schemas/workflow-draft.v1.json`) — the same schema
Studio's own canvas persistence validates against
(`@/lib/workflow-draft-contract`) and that `auditWorkflowDraft` checks every
Gao draft with. The envelope below wraps the draft with
`"format": "apxm_studio_workflow"` instead of a `schema_version` field; before
validation and Apply, Studio projects it onto the real document shape
(`schema_version: "apxm.workflow-draft.v1"`, position stripped, a synthesized
`capability_grants` covering only capabilities the draft itself uses) — see
`gaoWorkflowDraftToProject` / `structuralDraftDocument` in
`packages/widget-canvas/src/draft/workflow-draft.ts`.

**Do not re-derive the contract's required fields, closed node-kind enum, or
edge-kind enum by hand.** Studio injects them into context every turn as
schema-generated rules text
(`generateWorkflowDraftSchemaRules()` in
`packages/widget-canvas/src/draft/workflow-draft-schema-rules.ts`), mechanically
produced from the vendored `apxm.workflow-draft.v1` schema so it can't drift
from the contract. This file only covers the Gao-specific operating guidance
that JSON Schema can't express (wire envelope, live capability gating,
URL-verification discipline, prompt-token convention) — the same split
`workflow-draft-guide.ts` (`GAO_WORKFLOW_OPERATING_RULES`) makes.

## Required envelope

```json
{
  "format": "apxm_studio_workflow",
  "name": "workflow_id",
  "description": "short purpose",
  "nodes": [],
  "edges": []
}
```

## Node kinds

When the host supplies the canonical `nodeKinds` turn-context field, it contains the **live node-kind catalog** from the Studio registry. The allowed values come from the closed `DraftNodeKind` enum in `apxm.workflow-draft.v1` (`llm`, `acp_agent`, `synthesize`, `tool`, `memory_write`, `memory_read`, `merge`, `text`, `output`, and the trigger kinds — `cron_trigger`, `webhook_trigger`, `watch_trigger`, `channel_trigger`, `mcp_trigger`, `a2a_trigger`, `process_trigger`, `polling_trigger`). Use only `kind` values listed there.

Do not rely on a package-local snapshot. Check the injected **Studio node kinds** section, and treat the schema's enum as the source of truth if the two ever appear to disagree. If `nodeKinds` is absent, do not emit Apply workflow JSON; ask for host context or stay at the high-level planning stage.

## Tool / integration nodes

Studio lowers tool nodes to AIR `inv_cap` using **`capability`** and **`args`**:

```json
{
  "id": "fetch_source",
  "kind": "tool",
  "label": "Fetch Source",
  "config": {
    "capability": "http_get",
    "args": { "url": "<url verified by http_get in chat>" }
  }
}
```

Use only read capabilities listed under **Ready now**. Gao declares `http_get` for public HTTP research.

Capabilities that appear under **Needs connection** require connecting their provider in Integrations first. Do not propose disconnected provider capabilities in workflow drafts — use a **Ready now** capability such as `http_get` + LLM instead.

**URLs are not baked into Studio.** Before Apply workflow JSON, probe candidate endpoints with `http_get` in chat and use a URL that returned usable content for the operator's topic.

## LLM prompt tokens

- Upstream tool output: label slug in single braces, e.g. label **Fetch Source** → `{fetch_source}`
- Prefer `{{node_id}}` in drafts; Studio rewrites to label slugs at apply time
- Upstream LLM output: slug of that node's label
- Text block feeding an LLM: refer in prose as "the connected text block below" (backend inlines it)
- Trigger envelope: `{data}` or `{data.event.body}`

## Edges

`kind` is the closed `DraftEdge.kind` enum from the schema (default `data`):

- `data` — producer output feeds consumer prompt/args
- `control` — trigger starts downstream work
- `effect` — side-effect ordering

The graph must be acyclic — enforced the same way Studio's own canvas
persistence enforces it (`firstCycleNode`), and reported per the schema-generated
rules above, not a separate hand-rolled check here.

Wire `text` → `llm` (text inlines into the LLM prompt). Wire `tool` → `llm` when the LLM prompt references the tool's label token.

## Reference AIR shapes (not canvas JSON)

`examples/*.air` are not canvas JSON — they are vector fixtures
(`workspace/contracts/vectors/air/`, copied byte-for-byte) showing
the AIR a workflow ultimately lowers to once Studio compiles the canvas.
Useful when explaining *why* a node/edge shape matters, not as literal
`apxm_studio_workflow` blueprints:

- `approval_gate.air` — ask → pause → resume (human-approval barrier)
- `checkpoint_pipeline.air` — ask → checkpoint fence → ask (in-flow barrier without a full pause/resume round trip)
- `capability_pipeline.air` — register_capability → two chained `inv_cap` calls (the `tool` node's `config.capability`/`config.args` lowering)
- `converse_agent.air` — ask → autonomous (conversational loop) turn
- `multi_agent_delegate.air` — spawn_agent → delegate → communicate
- `runtime_agent_routing.air` — spawn_agent with `agent_route = "auto"` profile routing

Do not copy URLs, prompts, or domain content from these — they demonstrate
structure only. Discover live endpoints yourself via `http_get` during chat.

## Validation (Studio Apply)

When the operator clicks **Apply workflow**, Studio validates the draft against the same rules as the canvas editor:

- Node `kind` must exist in the live registry (injected each turn)
- Required `config.*` fields from the node spec
- Data edges must pass port compatibility (`edgeAllowed`)
- Tool capabilities must be **Ready now** in the session inventory
- Prompt tokens are auto-repaired, then re-validated before the canvas loads
