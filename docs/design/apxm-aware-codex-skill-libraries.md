# APXM-Aware Codex Skill Libraries

This document captures the integration direction for making Codex and other
coding agents APXM-aware. It connects the APXM "LLVM for agents" vision with a
practical skill-library architecture: skills should be discoverable,
convertible to AIR, exposable through `apxm-server`, executable through APXM,
and measurable against ordinary markdown skill usage.

The immediate goal is to separate implemented infrastructure from the remaining
runtime vision. APXM now has a server-local skill-library slice: configured
skill roots, a `SkillLibrary` scanner, manifest/hash validation, REST
list/get/validate routes, a narrow static `/v1/skills/{id}/execute` route, and
HTTP MCP skill tools. The remaining work is shared extraction beyond
`apxm-server`, GUI/CLI reuse, execution stores, streaming/node inspection,
provenance, full capability policy, and evaluation evidence from
human-readable skills through APXM artifacts.

## Why This Fits The Vision

The APXM vision is that agent systems need shared infrastructure: a clean IR,
modular optimization passes, portable artifacts, and open runtime contracts.
Skills are the right pressure test for that vision.

Today, most agent skills are prose packages. A `SKILL.md` file can be useful to a
host agent, but the host still interprets it ad hoc. APXM should make a skill
more like a software object:

- The source package remains readable and installable by normal agents.
- The APXM form lowers the skill into typed AIR.
- The compiler validates and optimizes the skill graph.
- The runtime executes it with explicit capabilities, state, sessions, metrics,
  and inspectable traces. Full replay needs scheduler snapshots and remains
  future work.
- The server exposes the library so Codex, GUI, MCP, and other clients see the
  same inventory.

That is the LLVM analogy applied to agent skills. Frontends can author or import
skills in different forms, but APXM gives them a shared IR, optimizer, artifact
format, and runtime.

## Current Integration Surfaces

APXM already has enough surface area to make this concrete.

| Surface | Current role | Relevance |
| --- | --- | --- |
| `.agents/skills` | `SkillResolver` scans repo-local skill directories containing `SKILL.md`. | Existing discovery convention for source skills. |
| Generated `AGENTS.md` | `ContextAssembler` renders Codex node context and references available skills. | Codex can be made APXM-aware per node without rewriting Codex. |
| ACP profiles | `apxm-acp` has profile templates including `codex`, permissions, env, and capability servers. | APXM can spawn Codex as a subprocess and provision capabilities. |
| `apxm-server` | HTTP gateway for execution, capabilities, agents, tasks, checkpoints, MCP, and A2A. | Correct home for a shared skill-library API. |
| `apxm-mcp-server` | Stdio MCP server exposes APXM compiler/runtime tools. | External agents can call APXM tools without shelling out. |
| Session artifacts | Execute/run paths emit session directories, metrics, traces, and node outputs. | Required evidence layer for evaluation and debugging. |
| Quality/eval tools | `quality_eval`, `ablation`, `trace_diff`, and `benchmark_e2e.py`. | Existing measurement machinery should be reused. |

The important architectural decision: skill libraries should be served by
`apxm-server`, not by the GUI. The GUI can display and manage skills, but the
server should own discovery, metadata, compilation status, capability checks,
and eventually dynamic loading. Codex and MCP clients should consume that same
server surface.

## Skill Package Model

APXM should distinguish source skills from APXM skills.

### Source Skill

A source skill is compatible with existing coding-agent ecosystems:

```text
skill-name/
  SKILL.md
  scripts/
  references/
  assets/
```

`SKILL.md` is the human contract: what the skill does, when to use it, required
steps, resources, and verification behavior. Scripts and references are support
files. Source skills are still useful as plain Codex skills.

### APXM Skill

An APXM skill is compiled or compilable:

```text
skill-name/
  skill.air
  skill.toml
  skill.apxmobj        # optional, for precompiled/static use
  resources/
  tests/
  conversion-report.json
```

`skill.toml` is the ABI manifest. A minimal manifest should include:

- `skill_id`, `version`, optional `display_name`, and `description`
- `entry_flow`
- source package path and source hash
- AIR hash and optional artifact hash
- compiler version, optimization level, and target
- expected inputs and outputs
- required capabilities and allowed tools
- side-effect policy and approval policy
- loading mode: `static`, `dynamic`, or `hybrid`
- compatibility with APXM runtime/compiler versions

The manifest lets the server answer "what is this skill?" without loading the
whole graph.

## Server-Owned Library API

`apxm-server` exposes skill libraries as a server-owned control plane. The
current implementation makes inventory, metadata, validation status, and narrow
static execution visible. It still does not execute arbitrary dynamic skills.
Skill identity is stable and machine-addressable: use `skill_id` plus `version`
in manifests and APIs, with `display_name` reserved for UI text.

Implemented REST surface:

| Endpoint | Purpose |
| --- | --- |
| `GET /v1/skills` | List discovered source and APXM skills. |
| `GET /v1/skills/{id}` | Return manifest, source metadata, hashes, capability requirements, and status. |
| `POST /v1/skills/{id}/validate` | Re-run validation for an installed skill and return structured failures. |
| `POST /v1/skills/{id}/execute` | Execute a precompiled, hash-validated, static `skill.apxmobj` with APXM-owned sessions. |
| `POST /v1/skills/{id}/execute/stream` | Execute a static skill and stream runtime events plus skill start/complete events. |
| `GET /v1/executions/{execution_id}` | Return an in-memory skill execution status/result record. |
| `GET /v1/executions/{execution_id}/nodes/{node_id}` | Return recorded node output detail for a skill execution. |

Future REST surface:

| Endpoint | Purpose |
| --- | --- |
| `POST /v1/skills/{id}/compile` | Compile `skill.air` to `.apxmobj`, with explicit opt level and target. |
| `GET /v1/skills/{id}/artifact` | Return artifact metadata or download handle when available. |
| Persistent execution store | Retain skill execution records beyond the current server process. |
| Rich node detail | Include node metrics, prompts, redacted summaries, and provenance beyond the current recorded output values. |

Implemented and planned MCP tools:

| MCP tool | Purpose |
| --- | --- |
| `apxm_skills_list` | Discover available APXM/source skills. |
| `apxm_skill_get` | Fetch one skill's manifest and status. |
| `apxm_skill_validate` | Validate a source or APXM skill. |
| `apxm_skill_call` | Execute a static APXM skill through the same manifest/hash/session checks as REST. |
| `apxm_skill_explain` | Summarize graph shape, capabilities, risks, and execution contract. |
| `apxm_skill_compile` | Later admin/dev tool for compiling a validated APXM skill. |

The server should use the same library implementation that the GUI and CLI use.
Avoid building separate skill scanners in each tool.

The GUI skill endpoint is still an empty placeholder. The server owns the real
skill surface today; GUI work should consume the server-owned model instead of
building another scanner.

Concrete current files:

- `crates/tools/apxm-server/src/main.rs` owns route registration.
- `crates/tools/apxm-server/src/execute.rs` compiles AIR and runs artifacts.
- `crates/tools/apxm-server/src/mcp.rs` exposes runtime capabilities as MCP
  tools.
- `crates/tools/apxm-gui/src/api/skills.rs` currently returns an empty skill
  list.
- `crates/orchestration/apxm-driver/src/skill_resolver.rs` discovers only
  `.agents/skills/**/SKILL.md` for node workspaces.
- `crates/orchestration/apxm-driver/src/session_output.rs` copies resolved
  source skills into per-node session directories.

The next implementation target is therefore:

1. Extract/reuse the skill scanner/model beyond `apxm-server`.
2. Have the GUI call the server-owned endpoint.
3. Add persistent execution stores and richer node-detail lookup.
4. Replace the current fail-closed static capability/tool policy with explicit
   capability admission and sandbox enforcement.

## Loading Modes

### Static Loading

Static loading compiles at install, CI, or release time:

```text
source skill -> skill.air -> skill.apxmobj -> registry entry
```

Use this for demos, CI, benchmark runs, and production deployments where the
artifact must be reproducible.

### Dynamic Loading

Dynamic loading resolves and prepares a skill at request time:

```text
request -> discover skill -> validate/convert -> compile/cache -> execute
```

This is the right long-term model for marketplaces and user-installed skill
libraries, but it must fail closed on missing capabilities, invalid AIR,
unsafe scripts, disallowed tools, or ambiguous conversion.

### Hybrid Loading

Hybrid loading serves metadata statically and compiles on first use. This is
likely the default for developer workflows: discovery is cheap, validation is
visible, and warm-cache runs behave like static loading.

## Why Execute Skills In APXM?

A prose skill is useful when the host agent only needs guidance. APXM execution
is useful when the skill needs runtime guarantees and evidence. The question is
not "can Codex read the steps?" but "which parts should be isolated,
observable, replayable, and optimizable?"

APXM execution buys:

- **Context separation:** each skill node gets explicit inputs and outputs
  instead of one large host-agent context window.
- **Node-level inspection:** sessions already persist per-node `node.json`,
  `prompt.txt`, `response.txt`, `output.json`, `metrics.json`, `status.json`,
  and `trace.ndjson` when enough graph metadata is available.
- **Runtime observability:** execution emits operation start/end, LLM token,
  token usage, scheduler, memory, checkpoint, and memoization events.
- **Optimization evidence:** O0 and O2 artifacts can be compared through
  compiler diagnostics, pass stats, eliminated ops, LLM call counts, tokens, and
  traces.
- **Isolation and policy:** compiled skill manifests can declare allowed tools,
  required capabilities, filesystem/network policy, and approval behavior before
  runtime admission.
- **Checkpointing and replay:** APXM already has checkpoint event payloads,
  pause/resume handlers, server checkpoint routes, and session directories; a
  skill server should attach those to skill runs explicitly.
- **Portable invocation:** Codex, Claude Code, GUI, MCP clients, and other
  agents can call the same APXM skill by manifest identity rather than copying
  different prose instructions into each host.

The cost is that execution requires a stable ABI and validation layer. APXM
should not execute every source skill by default. A source skill should remain a
markdown package until it has a reviewed `skill.toml`, a valid `skill.air`, and
passing tests.

## Runtime Execution Model

The runtime already has a usable foundation for compiled skills:

- `.apxmobj` supports generic extension sections in
  `crates/orchestration/apxm-artifact/src/lib.rs`.
- `Runtime::execute_artifact_with_session_and_emitter` in
  `crates/runtime/apxm-runtime/src/runtime.rs` loads an artifact, validates
  entry-flow arguments, registers artifact DAGs into the flow registry, executes
  with scheduler hooks, and returns node output maps and metrics.
- The runtime already parses one artifact sidecar section for Python tools in
  `python_tool_bridge_from_artifact`; skill manifests should follow the same
  pattern with an `apxm.skill_manifest` section.
- `FlowRegistry` in
  `crates/runtime/apxm-runtime/src/capability/flow_registry.rs` already stores
  compiled flows by agent/flow identity.
- `WORKFLOW_SPAWN` can execute artifact paths today, which is enough for the
  first static compiled-skill prototype.

The first execution path should be conservative:

```text
skill.toml + skill.air
  -> compile O0/O2 to skill.apxmobj
  -> attach apxm.skill_manifest section
  -> execute by artifact path
  -> emit session + per-node evidence
  -> compare O0 and O2 outputs/metrics
```

Only after this works should APXM add a named skill target:

```text
WORKFLOW_SPAWN(target_kind = "skill", target = "checkout-context-triage@0.1.0")
```

That named mode likely touches:

- `crates/core/apxm-core/src/types/execution/workflow.rs`
- `crates/runtime/apxm-runtime/src/executor/handlers/workflow_spawn.rs`
- `crates/orchestration/apxm-driver/src/runtime/workflow_spawn.rs`

An alternative invocation mode is a `SkillCapabilityExecutor`, so external
agents call `INV_TOOL(capability = "skill:checkout-context-triage")`. That is a
good MCP/server story, but artifact-path execution is the simpler first proof.

## Observability And Checkpoint Gaps

Top-level APXM runs already have useful evidence when session output is
enabled. The session layout can include:

- root `manifest.json`, `input.air`, `results.json`, `metrics.json`,
  `node_statuses.json`, `trace.ndjson`, and `live.json`
- per-node `node.json`, `live.json`, `status.json`, `metrics.json`,
  `trace.ndjson`, `output.json`, `prompt.txt`, and `response.txt`
- token accounting by total, node, flow, and agent where provider metrics are
  reported

That is enough for the first static A3/A4 compiled-skill experiment when the
skill runs as the top-level artifact. It is not yet enough for a general
"skill server" where skills can call nested skills and every nested node is
inspectable by skill identity.

Important missing pieces:

- Server-owned top-level runtime events now carry stable `skill_id`,
  `skill_version`, and entry-flow provenance. Artifact metadata, session
  manifests, and nested parent-skill provenance are still incomplete.
- Generic event streams now include typed `node_output`, `node_metrics`, and
  redacted `llm_prompt` events through `EmitterAdapter`. These payloads carry
  optional `node_name` when the runtime still has source node metadata,
  `node_output` carries a redacted summary/hash envelope, and server-owned
  top-level skill runs attach skill provenance to the event metadata.
- Child artifact workflow sessions can miss per-node directories because the
  driver does not pass reconstructed artifact graph metadata into the child
  session emitter.
- `FLOW_CALL` executes a child DAG and returns the sub-flow result, but it does
  not expose child `all_outputs` / `node_output_map` under a skill namespace.
- Session scope IDs exist in the lower-level emitter adapter and execution
  context, but the session writer does not persist them as a first-class
  isolation dimension.
- `CHECKPOINT`, `PAUSE`, and `RESUME` are useful control-flow pieces, but they
  are not full scheduler-state snapshots. Real skill checkpoint/replay needs a
  scheduler snapshot containing token values, node outputs, op states, ready and
  completed queues, graph/session/skill metadata, and enough backend state to
  resume safely.

The practical implication: first claim "APXM can observe and compare top-level
compiled skill runs." Do not claim "APXM can fully checkpoint and resume any
nested compiled skill library" until scheduler snapshots and skill-scoped
metadata are implemented.

## Gap Closure Backlog

The gaps above should close in this order. Each item is a shippable PR or small
PR stack with an explicit acceptance test. This backlog supersedes demo-workflow
work; old workflows are useful only as benchmark inputs.

### G1: Stable Skill Provenance

Problem: server-owned top-level runtime events carry `skill_id`,
`skill_version`, and entry flow, but artifact metadata, session manifests, and
nested parent-skill provenance are still incomplete.

Implementation:

- Done: add first-class skill provenance metadata to the core event envelope.
- Done: attach top-level server-owned skill provenance through
  `EmitterAdapter`.
- Add a shared `SkillManifest` / `SkillProvenance` model for artifact/session
  metadata.
- Add artifact section constants such as `apxm.skill_manifest.v1`.
- Preserve provenance in session manifests and metrics before changing core DAG
  wire format.
- Later, add stable skill fields to `DagMetadata` / `NodeMetadata` and artifact
  wire metadata when nested runs require it.

Likely files:

- `crates/orchestration/apxm-artifact/src/lib.rs`
- `crates/tools/apxm-server/src/skills.rs`
- `crates/tools/apxm-server/src/state.rs`
- `crates/orchestration/apxm-driver/src/session_output.rs`
- later: `crates/core/apxm-core/src/types/execution/dag.rs`
- later: `crates/core/apxm-core/src/types/execution/node.rs`
- later: `crates/orchestration/apxm-artifact/src/wire.rs`

Acceptance tests:

- Loading a skill artifact records `skill_id`, `skill_version`, artifact hash,
  opt level, and entry flow in the session manifest.
- Two versions of the same skill can be registered without metadata collision.
- A mismatched manifest/artifact hash rejects before execution.

### G2: Typed Node Output Events

Problem: `SessionEventEmitter` writes node outputs and prompts to files, and the
generic event stream now includes typed redacted `llm_prompt`, `node_output`,
and `node_metrics`; server-owned top-level skill runs attach `skill_id`,
`skill_version`, and entry flow. Those payloads now carry optional node names
when runtime metadata is available, but nested parent provenance is still
incomplete.

Implementation:

- Done: add core `node_output` event kind and payload.
- Done: add core `node_metrics` event kind and payload.
- Done: implement `emit_node_output` in `EmitterAdapter`.
- Done: implement `emit_node_metrics` in `EmitterAdapter`.
- Done: add core event kind and payload for redacted `llm_prompt`.
- Done: implement `emit_llm_prompt` in `EmitterAdapter`.
- Done: stream summary/hash/redaction metadata instead of raw node output
  values.
- Done: include optional `node_name` on `llm_prompt`, `node_output`, and
  `node_metrics` payloads when runtime node metadata is available.
- Keep `skill_id` / `skill_version` / `flow_name` in event metadata rather than
  duplicating it into every payload. `scope_id` already travels in the event
  envelope when set by the runtime.
- Keep full output values in session files when policy allows; stream summaries
  by default.

Likely files:

- `crates/core/apxm-core/src/events/kind.rs`
- `crates/core/apxm-core/src/events/payload.rs`
- `crates/runtime/apxm-runtime/src/executor/events.rs`
- `crates/runtime/apxm-runtime/src/executor/emitter_adapter.rs`
- `crates/orchestration/apxm-driver/src/session_output.rs`

Acceptance tests:

- Runtime emitter tests and skill stream tests receive typed `node_output`
  events.
- Runtime emitter tests and skill stream tests receive typed `node_metrics`
  events.
- LLM nodes emit prompt summaries and token events under the same node id.
- Redaction mode suppresses full values but preserves value hashes.

### G3: Child Artifact Session Metadata

Problem: child artifact workflow sessions can miss per-node directories because
the driver passes no reconstructed graph metadata into the child session
emitter.

Implementation:

- Reconstruct an `AirModule`-like metadata view from the child artifact's entry
  DAG or DAG list.
- Pass that metadata into `create_session_writer` and `create_session_emitter`
  for artifact-path workflow spawns.
- Ensure child artifact sessions write `input.air` or an equivalent
  `input-artifact.json` metadata file.

Likely files:

- `crates/orchestration/apxm-driver/src/runtime/workflow_spawn.rs`
- `crates/orchestration/apxm-driver/src/session_output.rs`
- `crates/tools/apxm-cli/src/commands/execute.rs`

Acceptance tests:

- Parent `WORKFLOW_SPAWN artifact_path` run creates a child session.
- Child session contains per-node directories, `output.json`, `status.json`,
  and `trace.ndjson`.
- Child session records the child artifact hash and parent execution id.

### G4: FLOW_CALL Child Output Namespace

Problem: `FLOW_CALL` runs a child DAG and returns the sub-flow result, but it
does not expose child `all_outputs` / `node_output_map` under a skill namespace.

Implementation:

- Propagate runtime scheduler config into child flow calls instead of using a
  bare default scheduler.
- Collect child output maps when parent execution is collecting all outputs.
- Store child outputs under a namespace such as
  `flow::<agent>.<flow>` or `skill::<skill_run_id>/<flow>`.
- Emit child output summaries through typed node-output events.

Likely files:

- `crates/runtime/apxm-runtime/src/executor/handlers/flow_call.rs`
- `crates/runtime/apxm-runtime/src/scheduler/dataflow.rs`
- `crates/runtime/apxm-runtime/src/executor/hooks.rs`
- `crates/runtime/apxm-runtime/src/runtime.rs`

Acceptance tests:

- A parent graph calls a registered child flow with two output-producing nodes.
- Runtime result includes parent output plus child namespaced outputs.
- Session/effect ledger can distinguish parent node outputs from child flow
  node outputs.

### G5: Scope Persistence

Problem: scope IDs exist in the lower-level emitter adapter and execution
context, but the session writer does not persist them as a first-class
isolation dimension.

Implementation:

- Implement `set_current_scope_id` / `current_scope_id` in
  `SessionEventEmitter`.
- Stamp scope id into root trace events, node trace events, session manifest,
  node metadata, and skill-run indices.
- Add a `skills/<skill_run_id>/` or `scopes/<scope_id>/` index that maps scopes
  to node ids and outputs.

Likely files:

- `crates/orchestration/apxm-driver/src/session_output.rs`
- `crates/runtime/apxm-runtime/src/executor/context.rs`
- `crates/runtime/apxm-runtime/src/executor/emitter_adapter.rs`
- `crates/core/apxm-core/src/events/mod.rs`

Acceptance tests:

- Nested child execution emits a distinct scope id.
- Session manifest does not write `scope_id: None` for scoped runs.
- Concurrent runs of the same skill do not share scope ids or node-output
  indices.

### G6: Scheduler Snapshot Checkpointing

Problem: `CHECKPOINT`, `PAUSE`, and `RESUME` are useful control-flow pieces, but
they are not full scheduler-state snapshots.

Implementation:

- Define a scheduler checkpoint model containing token values, node outputs,
  op states, ready/running/completed queues, retry state, delegated/promise
  tokens, graph/session/skill metadata, and AAM/memory references.
- Add a scheduler snapshot API and a restore API.
- Make `CHECKPOINT` use scheduler snapshots rather than only handler inputs.
- Make `PAUSE` and `RESUME` use the same checkpoint format.
- Mark side-effectful nodes as non-replayable unless the manifest declares
  them idempotent.

Likely files:

- `crates/runtime/apxm-runtime/src/scheduler/state.rs`
- `crates/runtime/apxm-runtime/src/scheduler/dataflow.rs`
- `crates/runtime/apxm-runtime/src/executor/handlers/checkpoint.rs`
- `crates/runtime/apxm-runtime/src/executor/handlers/pause.rs`
- `crates/runtime/apxm-runtime/src/executor/handlers/resume.rs`
- `crates/runtime/apxm-runtime/src/aam/session.rs`

Acceptance tests:

- Checkpoint after node N, resume, and produce the same final output as an
  uninterrupted run.
- Replay skips completed idempotent nodes and refuses to duplicate
  non-idempotent side effects.
- Missing backend/capability state fails with a clear resume error.
- Checkpoint/replay claims are blocked unless this scheduler snapshot evidence
  exists.

## Clear Path Forward

The path should be staged so each step is independently useful and testable.
The core principle: execute **static, reviewed APXM skill artifacts first**;
add named dynamic skill loading only after the manifest, server, sandbox, and
observability contracts are real.

### Step 1: Define A Skill Manifest, Not A Loader

Add a minimal manifest model before adding any server execution.

Suggested first type:

```rust
pub struct SkillManifest {
    pub skill_id: String,
    pub version: String,
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub entry_flow: String,
    pub source_hash: Option<String>,
    pub air_hash: Option<String>,
    pub artifact_hash: Option<String>,
    pub required_capabilities: Vec<String>,
    pub allowed_tools: Vec<String>,
    pub isolation: SkillIsolationPolicy,
    pub inputs: Vec<SkillParam>,
    pub outputs: Vec<SkillOutput>,
}
```

Implementation target:

- Use the shared `crates/core/apxm-skill` crate for manifest identity, hashes,
  validation reports, and execution provenance.
- Keep filesystem scanning, HTTP handlers, and static artifact admission in
  `crates/tools/apxm-server/src/skills.rs`.
- Recognize `skill.toml`, `skill.air`, and optional `skill.apxmobj`.
- Validate hashes and required files, but do not execute anything yet.

Tests:

- Valid manifest parses.
- Missing entry flow fails.
- Wrong artifact hash fails.
- Unknown required capability is reported as unavailable.

### Step 2: Expose A Read-Only Skill Registry

Make APXM visible as a skill server before it can execute skills.

Server API:

```text
GET /v1/skills
GET /v1/skills/{id}
POST /v1/skills/{id}/validate
```

Implementation target:

- Add an explicit server skill-root configuration surface first, such as
  repeated `--skill-root` flags plus an `APXM_SKILL_ROOTS` fallback. REST
  callers should not be able to pass arbitrary roots or paths.
- `crates/tools/apxm-server/src/skills.rs`
- `crates/tools/apxm-server/src/state.rs`
- `crates/tools/apxm-server/src/main.rs`
- later, `crates/tools/apxm-gui/src/api/skills.rs` should consume this server
  model instead of returning an empty list.

This step is safe because it only reads manifests and artifacts. It gives Codex,
Claude Code, MCP clients, and the GUI a shared inventory.

### Step 3: Execute Static Top-Level Skill Artifacts

Add execution only for precompiled, hash-validated `.apxmobj` files.

Server API:

```text
POST /v1/skills/{id}/execute
POST /v1/skills/{id}/execute/stream
GET /v1/executions/{execution_id}
GET /v1/executions/{execution_id}/nodes/{node_id}
```

Execution sequence:

```text
resolve skill id
  -> validate manifest and artifact hash
  -> Artifact::from_bytes
  -> find @entry DAG
  -> validate args against manifest and entry params
  -> create APXM-owned skill session dir
  -> create in-memory execution record
  -> Runtime::execute_artifact_with_session_and_emitter
  -> return execution id, result, stats, and session path
```

Implemented in the first static slice:

- Reuse `Runtime::execute_artifact_with_session_and_emitter`.
- Reject raw AIR, client roots, client session roots, symlinked artifacts,
  mismatched hashes, entry-flow mismatches, invalid session ids, non-static
  operations, and capability/tool-declaring manifests until full policy exists.
- Return the standard `ExecuteResponse` plus `execution_id`.
- Stream skill start/runtime/complete events on the static stream endpoint.
- Record typed node outputs in the in-memory execution store and expose them
  through node-detail lookup.

Next implementation target:

- Replace the in-memory execution store with bounded retention or persistent
  storage.
- Do not accept raw AIR through the skill execution endpoint.
- Require `SchedulerConfig.collect_all_outputs = true` for skill executions
  where node-output inspection is promised.
- Initialize server runtime consistently with the driver. If direct reuse would
  invert dependencies, extract a shared runtime-builder module/crate for LLM
  registry, capability registry, sandbox registry, model router, middlewares,
  agent registry, inner-plan linker, and workflow spawner setup.

Tests:

- Static pure artifact executes and returns expected output.
- Mismatched hash rejects before execution.
- Missing required capability rejects before execution.
- SSE emits operation events and final result.
- Execution-detail routes return state from the execution store instead of
  scanning arbitrary client-supplied paths.

### Step 3.5: Harden The Agent-Facing Server Boundary

The skill server must be narrower than the existing developer execution API.
`/v1/execute` can remain a dev/debug endpoint for raw AIR, but agent-facing
clients should call skills by manifest identity only.

Required guardrails before exposing skill execution to Codex, Claude Code, MCP,
or other agents:

- Do not accept arbitrary AIR on `/v1/skills/{id}/execute`.
- Do not let clients choose arbitrary `session_root`; create skill sessions
  under APXM-owned session storage.
- Require token budgets and timeout limits for every skill call.
- Validate JSON/object arguments against the skill ABI before execution.
- Reject missing or undeclared capabilities before runtime admission.
- Treat `/v1/capabilities/register` as admin-only or disabled in skill-server
  mode.
- Do not expose every skill as an MCP tool by default. Use one generic
  `apxm_skill_call` MCP tool first, with server-side policy checks.
- Prefer OS-level sandbox backends for side-effectful skills. The process
  fallback is policy-only isolation and should be reported as degraded.
- Add auth before binding a skill server outside a trusted localhost/dev
  context; the current server uses permissive CORS.

Server runtime initialization should also match the driver path. A bare
`Runtime::new(RuntimeConfig::default())` does not configure the same LLM
registry, capability registry, sandbox registry, model router, middlewares,
agent registry, or workflow spawner as `RuntimeExecutor::new`. Extract or reuse
that initialization path before claiming isolated server execution.

### Step 4: Make Node Outputs First-Class Events

The first static skill server can read node files from the session directory,
but a proper multi-client server needs typed event payloads.

Implementation target:

- Done: add core `node_output` event kind/payload and forward it from
  `crates/runtime/apxm-runtime/src/executor/emitter_adapter.rs`.
- Done: add core `node_metrics` event kind/payload and forward it from
  `crates/runtime/apxm-runtime/src/executor/emitter_adapter.rs`.
- Done: add core event kind/payload for redacted `llm_prompt`.
- Done: implement prompt forwarding in
  `crates/runtime/apxm-runtime/src/executor/emitter_adapter.rs`.
- Include optional `node_name` in node event payloads when runtime metadata is
  available; `skill_id`, `skill_version`, `flow_name`, `node_id`, `scope_id`,
  value summary, hash, and redaction metadata are already present in the stream
  path.
- Keep full values in session files when policy allows; stream summaries by
  default.
- Ensure root and per-node `trace.ndjson` files also receive typed
  `node_output`, `node_metrics`, and redacted `llm_prompt` events. File output
  alone is not enough for replayable observability.

Tests:

- `/v1/skills/{id}/execute/stream` emits node-output events for a pure graph.
- Redaction policy can suppress full node values.
- Node output event order matches operation lifecycle order.

### Step 5: Add Skill Identity To Artifacts And Sessions

Before nested skills, APXM needs provenance.

Implementation target:

- Add skill provenance to artifact sections first:
  `apxm.skill_manifest`.
- Then preserve stable skill metadata in DAG/node/session views:
  `skill_id`, `skill_version`, `skill_run_id`, `flow_name`, `parent_skill_id`,
  and `scope_id`.
- Wire this through artifact load, runtime context, event metadata, session
  manifests, and node directories.

Tests:

- Session root records skill id/version/artifact hash.
- Every node event in a skill run has a skill run id or inherited scope.
- Two concurrent executions of the same skill do not mix node outputs.

### Step 6: Support Nested And Named Skill Invocation

Only after top-level static artifact execution is observable should APXM add
named invocation.

Candidate APIs:

```text
WORKFLOW_SPAWN(target_kind = "skill", target = "checkout-context-triage@0.1.0")
INV_TOOL(capability = "skill:checkout-context-triage")
```

Implementation target:

- Extend workflow targets in
  `crates/core/apxm-core/src/types/execution/workflow.rs`.
- Extend workflow spawning in
  `crates/runtime/apxm-runtime/src/executor/handlers/workflow_spawn.rs`.
- Extend driver workflow spawning in
  `crates/orchestration/apxm-driver/src/runtime/workflow_spawn.rs`.
- Preserve child artifact graph metadata so child node directories and outputs
  are written.
- For `FLOW_CALL`, define a result-type contract for child outputs before
  exposing namespaced data. Current node-output maps are keyed by numeric node
  ids, so nested results need either a structured `child_outputs` field or a
  typed output-key model rather than ad hoc string keys.
- Once the contract exists, propagate child outputs under a skill/flow namespace
  instead of discarding `all_outputs` and `node_output_map`.

Tests:

- Parent graph invokes a named skill.
- Child skill nodes appear under the session with skill identity.
- Parent can inspect child result without receiving unrelated child context.

### Step 7: Add Real Skill Checkpointing

Checkpointing should be the last major claim, not the first.

Current checkpoint/pause/resume pieces are useful, but they do not yet capture
full scheduler state. Real checkpoint/replay needs:

- token values
- node output map
- op statuses
- ready/running/completed queues
- retry state
- delegated promise tokens
- graph/session/skill metadata
- enough backend state to avoid unsafe duplicate side effects

Implementation target:

- Add a scheduler snapshot API around scheduler state.
- Make `CHECKPOINT` a scheduler-owned barrier or give it access to that
  snapshot.
- Make `PAUSE`/`RESUME` use the same checkpoint format.
- Emit checkpoint saved/restored events with skill scope metadata.

Tests:

- Checkpoint after node N, resume, and produce the same final output.
- Replay does not rerun side-effectful nodes unless the manifest marks them
  idempotent.
- Failed resume reports missing backend/capability state clearly.

### Step 8: Run The A0-A4 Skill Evaluation

Do not market this as a broad win until this matrix exists.

First fixture:

```text
examples/python/skill-evals/corpus/checkout-context-triage/
```

Arms:

- A0: no skill
- A1: markdown `SKILL.md` with explicit pasos
- A2: readable `skill.air` / APXM format supplied to host agent
- A3: APXM static artifact O0
- A4: APXM static artifact O2

Success criteria:

- A3 preserves A1 correctness on the triage task.
- A4 preserves A3 correctness.
- A4 has artifact-backed evidence of fewer calls/tokens or eliminated ops.
- Every claim is backed by CSV rows, session metrics, compiler diagnostics, or
  trace/session artifacts.

## Skill Conversion Pipeline

APXM should have a `skill-to-air` frontend. It can be assisted by an agent, but
the load-bearing output must be structured and auditable.

Recommended pipeline:

1. Parse `SKILL.md` frontmatter and markdown with structured parsers.
2. Normalize into a `SkillSpec`: name, triggers, anti-triggers, prerequisites,
   ordered steps, decision points, resources, scripts, expected outputs, and
   verification steps.
3. Map the spec to AIS:
   instructions become reasoning nodes, scripts become registered
   capabilities, references become explicit resource reads, checks become
   verification/tool nodes, branches become typed control flow, and joins
   become explicit synchronization.
4. Emit `skill.air` and `skill.toml`.
5. Run validation, O0 compile, O2 compile, decompile, and round-trip checks.
6. Write `conversion-report.json` with unmapped sections, inferred decisions,
   assumptions, missing capabilities, and risk flags.

The converter must not silently turn vague prose into production AIR. Any
LLM-inferred graph structure is provisional until validation and tests approve
it.

## Codex Integration Model

Codex does not need to be rewritten to become APXM-aware. The first integration
layer should be additive:

1. Repo-root instructions teach Codex APXM conventions: prefer `dekk apxm ...`,
   ignore `external/vllm` unless asked, use sessions and metrics for debugging,
   and prefer compile-only checks when no live backend is reachable.
2. Repo-local skills teach APXM workflows: build, test, debug, benchmark,
   compiler pass work, server/MCP work, and quality evaluation.
3. APXM-generated node workspaces provide task-specific `AGENTS.md` and copied
   skills for spawned Codex nodes.
4. APXM MCP tools let Codex validate AIR, inspect the AIS contract, compile
   graphs, and eventually query the skill library.
5. APXM runtime executes compiled skill graphs when the task needs orchestration,
   evidence, replay, or optimization.

This keeps Codex as the shell and APXM as the execution substrate. That matches
the vision's "wrap, do not rewrite" path.

## Initial APXM Skill Pack

The first repo-local APXM-aware skill pack should focus on workflows that
currently require local project knowledge:

| Skill | Purpose |
| --- | --- |
| `apxm-orient` | Read the right docs and crate boundaries before changing code. |
| `apxm-author-python` | Author Python frontend workflows and inspect emitted AIR. |
| `apxm-compiler-pass` | Develop or debug AIS/MLIR compiler passes. |
| `apxm-runtime-backends` | Work on runtime scheduling, backend routing, and vLLM hints. |
| `apxm-server-mcp` | Modify REST/MCP/A2A server surfaces coherently. |
| `apxm-quality-eval` | Run quality fixtures, budgets, judges, and claim checks. |
| `apxm-demos-benchmarks` | Work on Gemma demos, benchmark harnesses, and reports. |
| `apxm-design-docs` | Update conceptual docs without overclaiming implementation state. |

The first batch should be `apxm-orient`, `apxm-server-mcp`,
`apxm-quality-eval`, `apxm-compiler-pass`, and `apxm-demos-benchmarks`.

## Evaluation Matrix

The evaluation should compare ordinary skill use against APXM representations
and APXM execution. Use the same prompt, model/backend, workspace seed, tool
policy, budget, and timeout across arms.

| Arm | Name | What runs | Question |
| --- | --- | --- | --- |
| A0 | No skill | Baseline coding agent receives only the task. | Does any skill help? |
| A1 | Markdown skill | Host agent follows `SKILL.md`. | How good is the existing skill ecosystem? |
| A2 | AIR-as-skill | Host agent receives readable `skill.air`. | Is AIR useful as a portable skill representation? |
| A3 | APXM O0 | APXM executes compiled `skill.air` without optimization. | Does executable APXM preserve correctness? |
| A4 | APXM O2 | APXM executes optimized `skill.air`. | Do compiler/runtime optimizations improve cost, speed, or auditability? |

Interpret deltas carefully:

- `A1 - A0`: skill value.
- `A2 - A1`: readability/portability of AIR as a skill artifact.
- `A3 - A1`: executable parity.
- `A4 - A3`: compiler/runtime value.
- `A4 - A0`: end-to-end APXM-aware skill value.

Reuse the existing Gemma skill evaluation plan and benchmark harness rather
than creating a parallel metrics stack.

## Benchmark Ladder

Use a ladder, not one giant benchmark. Each level answers a different question
and has different claim boundaries.

| Level | Benchmark family | Why it matters | What it can prove | What it cannot prove |
| --- | --- | --- | --- | --- |
| L0 | Offline structural checks | Fast CI and local safety. | Manifests parse, AIR validates, artifacts decode, tests and scorers work. | User-facing quality, latency, or model behavior. |
| L1 | Compiler microbenchmarks | Isolate compiler pass value. | O2 removes ops/calls/tokens or emits scheduling/cache metadata on controlled graphs. | That a real agent task gets better. |
| L2 | First A0-A4 skill fixture | Measures the core product hypothesis. | Markdown vs AIR vs APXM O0/O2 on one skill with task assertions. | Broad benchmark generality. |
| L3 | Skill-server isolation benchmarks | Proves APXM can safely host skills. | Manifest validation, capability denial, sandbox policy, per-run sessions, multi-client isolation. | Better model answers. |
| L4 | Nested skill observability benchmarks | Proves skill composition is inspectable. | Parent/child skill node outputs, scopes, traces, and session layout. | Full checkpoint/replay unless scheduler snapshots exist. |
| L5 | External coding/terminal benchmarks | Tests usefulness in real agent work. | Impact on test-based coding and terminal tasks. | Tool-policy or GUI/desktop behavior. |
| L6 | External tool-state and desktop benchmarks | Tests policy, state, and UI-heavy tasks. | Tau-bench/OSWorld/GAIA-style skill usefulness after tool and GUI surfaces mature. | First-milestone APXM skill execution. |

### L0: Offline Structural Checks

Use this in every PR. These checks should not require a live LLM backend.

Benchmarks and commands:

```bash
python -m pytest tools/quality_eval/tests -q
bash tools/eval_harness/tests/test_trace_diff.sh
cargo test -p apxm-artifact --locked
cargo check -p apxm-cli --features driver,metrics --locked
```

Use `tests/quality_fixtures/optimisation_invariant` as the first deterministic
execution smoke. It has a `golden_output.txt`, should require zero LLM calls,
and is the fastest way to prove that O0/O2 preserve output before introducing
model variability:

```bash
dekk apxm quality-eval -- --fixture optimisation_invariant --opt 0
dekk apxm quality-eval -- --fixture optimisation_invariant --opt 2
```

For skill packages, add backend-free checks:

```bash
dekk apxm validate examples/python/skill-evals/corpus/<skill>/apxm/skill.air
dekk apxm compile examples/python/skill-evals/corpus/<skill>/apxm/skill.air \
  -O 0 \
  -o /tmp/<skill>-O0.apxmobj
dekk apxm compile examples/python/skill-evals/corpus/<skill>/apxm/skill.air \
  -O 2 \
  -o /tmp/<skill>-O2.apxmobj \
  --emit-diagnostics /tmp/<skill>-O2.diagnostics.json
```

Why: this catches broken manifests, broken AIR, broken artifact encoding, and
broken harness logic before runtime/model variability enters the loop.

### L1: Compiler Microbenchmarks

Use `examples/python/benchmarks/stress/` to measure specific compiler/runtime
mechanisms under controlled graphs:

| Source | Purpose | Relevant claim |
| --- | --- | --- |
| `dead_context_stress.py` | Dead context operands and unused branches. | DCE removes unused context and can reduce LLM calls/tokens. |
| `cse_stress.py` | Common subexpression elimination. | Explicit CSE pass reduces duplicated graph work. |
| `shared_prefix_fanout.py` / `prefix_fanout_large.py` | Shared prefix reuse opportunities. | Compiler emits prefix/cache hints; backend counters must move before cache claims. |
| `priority_scheduling.py` / `mixed_priority.py` | Runtime scheduling priority. | Critical-chain finish or queue wait improves under pressure. |
| `memo_cache_stress.py` | Runtime memoization. | Repeated calls hit memo cache without changing output. |
| `chained_llm.py` | Sequential critical path. | Optimization does not invent parallelism where dependencies are serial. |
| `demo_code_critique.py` | Composite review-style graph. | End-to-end benchmark source, only after fresh metrics are captured. |

Commands:

```bash
python3 examples/python/benchmarks/benchmark_e2e.py \
  --graph examples/python/benchmarks/stress/dead_context_stress.py \
  --compile-only \
  --opt-level 0 \
  --opt-level 2 \
  --emit-compiler-diagnostics \
  --output /tmp/apxm-bench-dead-context.csv \
  --diagnostics-dir /tmp/apxm-bench-dead-context-diagnostics

cd tools && python3 -m ablation
cd tools && python3 -m ablation.conflict_matrix
```

Why: L1 benchmarks are the right place to prove compiler pass behavior. They
are not enough to claim a better coding agent or a better skill library.

### L2: A0-A4 Skill Benchmarks

This is the central skill evaluation. Every task fixture should run:

- A0: no skill
- A1: markdown `SKILL.md` with explicit steps
- A2: readable `skill.air` / APXM format supplied to the host agent
- A3: static APXM artifact at O0
- A4: static APXM artifact at O2

First fixture:

```text
examples/python/skill-evals/corpus/checkout-context-triage/
```

Why this fixture first:

- It is understandable and small.
- It has a positive trigger and obvious negative triggers.
- It already maps to `02_checkout_context_pruning.py`.
- It has existing O0/O2 evidence: O0 keeps six LLM calls; O2 removes four dead
  context branches and runs two calls.
- It tests the exact APXM promise: context separation plus optimization without
  quality regression.

Minimum task set:

| Task | Purpose |
| --- | --- |
| `duplicate-timeout.toml` | Positive trigger; checkout triage should activate. |
| `negative-api-auth.toml` | Negative trigger; checkout skill should not activate. |
| `ambiguous-incident.toml` | Tests clarification behavior and trigger precision. |
| `noisy-context.toml` | Tests irrelevant context rejection. |

Required scoring assertions:

- root cause tied to duplicate `order_authorized` timeout/retry behavior
- idempotency key distinction is present
- hotfix avoids database migration
- regression test covers provider timeout/retry duplication
- unrelated API/observability/compliance/rollout facts are not used
- skill activation matches fixture expectation

How:

```bash
python3 examples/python/skill-evals/scripts/run_skill_case.py \
  --case checkout-context-triage \
  --arms A0 A1 A2 A3 A4 \
  --iterations 5 \
  --run-dir /tmp/apxm-skill-evals/checkout-context-triage \
  --interleave-arms
```

Until `run_skill_case.py` exists, use `benchmark_e2e.py` directly for A3/A4
only for compile/runtime smoke. Do not make A0/A1/A2 comparison claims until a
host-agent runner records prompts, outputs, budgets, model/backend labels,
timeouts, and quality scores through the shared result schema.

Recommended internal benchmark order:

1. `tests/quality_fixtures/optimisation_invariant`: deterministic no-regression
   and zero-token smoke.
2. `tests/quality_fixtures/qa_factual`, `no_hallucination`,
   `code_completion`, and `summarize_short`: quality-contract fixtures with
   existing rubrics and budgets.
3. `benchmark_e2e.py --precompile-artifacts` on
   `optimisation_invariant/graph.air`: proves artifact-run mechanics.
4. `benchmark_e2e.py --precompile-artifacts` on
   `02_checkout_context_pruning.py`: first meaningful APXM O0/O2 skill win.
5. `01_review_synthesis_skill.py`: complex skill-shaped workflow evidence.
6. `03_vllm_backend_hints.py`: runtime/backend scheduling evidence only when
   vLLM priority and queue pressure are part of the claim.

### L3: Skill-Server Isolation Benchmarks

These benchmarks prove that APXM can be a server of isolated skills.

Scenarios:

| Scenario | Expected behavior |
| --- | --- |
| Valid pure skill artifact | Runs and writes server-owned session output. |
| Corrupt artifact hash | Rejected before runtime execution. |
| Missing required capability | Rejected before runtime execution. |
| Disallowed tool/capability | Rejected even if runtime has the capability globally. |
| Disallowed filesystem write | Fails under sandbox policy. |
| Network-denied skill | Cannot make network calls unless manifest allows it. |
| Parallel clients | Two runs of same skill get distinct `run_id`, session, scope, and node outputs. |
| Arbitrary session root attempt | Client cannot write sessions outside APXM-owned storage. |
| Raw AIR through skill endpoint | Rejected; raw AIR remains dev-only `/v1/execute`. |

Why: this is the benchmark family that supports the "APXM as skill server"
claim. It does not measure answer quality; it measures trust boundaries.

Concrete tests to add before claiming server isolation:

```bash
cargo test -p apxm-server server_execute_inv_uses_configured_sandbox_backend
cargo test -p apxm-server server_execute_inv_rejects_missing_sandbox_backend
cargo test -p apxm-server mcp_tools_call_sandboxed_capability
cargo test -p apxm-server server_policy_denies_disallowed_bash_before_sandbox
cargo test -p apxm-server server_execute_stream_emits_ordered_runtime_events_and_execute_complete
cargo test -p apxm-server server_same_session_serializes_different_sessions_parallel
cargo test -p apxm-server task_queue_concurrent_claims_are_unique
cargo test -p apxm-driver --test sandbox_e2e sandbox_backend_strips_sensitive_env_overrides
```

Linux-only OS-isolation checks should use bubblewrap or a stronger backend:

```bash
cargo test -p apxm-driver --test sandbox_e2e bubblewrap_denies_write_outside_declared_paths
```

Do not count the process fallback as OS isolation. It is useful for policy and
timeout enforcement, but it cannot enforce filesystem or network isolation.

Server risks to keep out of the skill benchmark surface:

- current `/v1/execute` accepts raw AIR and arbitrary `session_root`
- current server CORS is permissive
- dynamic `/v1/capabilities/register` is too powerful for untrusted clients
- stdio MCP execution paths that create a bare runtime do not prove sandboxed
  skill execution
- skill-server mode needs the driver-equivalent runtime setup path before
  isolation claims are valid

### L4: Nested Skill Observability Benchmarks

These benchmarks should come after top-level static skill execution.

Scenarios:

| Scenario | Expected evidence |
| --- | --- |
| Parent graph spawns child artifact by `artifact_path` | Parent and child sessions exist; child result is visible. |
| Parent invokes named skill target | Child nodes have `skill_id`, `skill_run_id`, `scope_id`, and parent link. |
| Child flow emits multiple node outputs | All child outputs are visible under one skill invocation. |
| Child skill fails one node | Failure is attributed to child skill/node without corrupting parent evidence. |
| Nested O0/O2 comparison | O2 optimization evidence is attached to the child skill artifact/run. |

Why: this is where APXM proves composition, tracking, and observability. Do not
use L4 to claim checkpoint/replay until scheduler snapshots are implemented.

### L5: External Coding And Terminal Benchmarks

After L2 and L3 are stable, add small external benchmark subsets.

Use these first:

| Family | Why | How A0-A4 maps |
| --- | --- | --- |
| Terminal-Bench-style tasks | Measures command-line tool work with objective grading. | Same A0-A4 arms, with APXM controlling/replaying a terminal workflow skill. |
| SWE-bench-style coding repair | Coding agents are the target users; success is test-based. | A0 no skill, A1 markdown debugging/TDD skill, A2 AIR skill, A3/A4 APXM skill guiding or executing structured workflow. |

Inclusion criteria:

- task has deterministic tests or objective final-state checks
- task can run in a local sandbox
- task does not require secrets or uncontrolled network access
- expected success can be measured without subjective judging
- runtime artifacts can be archived without leaking sensitive data

Why not start with these: they are expensive and noisy. First prove the APXM
skill mechanism on internal fixtures, then use external tasks for ecological
validity.

Use Terminal-Bench-style tasks before SWE-bench-style coding repair because
terminal tasks align directly with APXM's command execution, sandboxing,
session artifacts, and checkpoint direction. For coding repair, pin the exact
benchmark variant. As of April 28, 2026, SWE-bench Verified remains useful for
comparability, but current frontier-model claims should also consider
SWE-bench Pro or a private/held-out coding set because benchmark saturation and
contamination risk are now part of the coding-benchmark landscape.

### L6: Tool-State, Desktop, And General-Agent Benchmarks

Use these later:

| Family | Use when | Why |
| --- | --- | --- |
| Tau-bench-style tasks | APXM skill policies need to prove tool-state correctness. | Final database/tool state and policy compliance matter more than text quality. |
| OSWorld-style tasks | APXM has mature GUI/computer-use actions. | Tests visual/desktop workflows and OS actions. |
| GAIA-style tasks | APXM needs general multi-step research/tool reasoning evidence. | Useful for broad assistant tasks, but grading is less direct than coding/terminal tests. |

These should not gate the first compiled-skill server. They belong after APXM
has skill manifests, server execution, policy isolation, and traceable tool
state.

## Evaluation Methodology

Every benchmark run should produce an artifact bundle:

```text
reports/<run-id>/
  manifest.json
  results.csv
  config.toml
  skills/<skill-id>/skill.toml
  skills/<skill-id>/skill.air
  artifacts/
  sessions/
  diagnostics/
  claim-report.md
```

Rules:

- Interleave arms: run A0/A1/A2/A3/A4 trial 1 before trial 2.
- Use at least 5 iterations for headline A0-A4 claims; 3 is acceptable for
  smoke runs.
- Keep workspace seed, prompt, backend/model, tool policy, timeout, token
  budget, and evaluator identical across arms.
- Separate compile time, load time, runtime, and host-agent wall time.
- Report medians and ranges; include mean only with variance.
- For quality claims, require success parity before claiming cost/speed wins.
- For speed claims, prefer graph duration, critical milestone finish, or
  observed critical path over whole-process wall time.
- For token/cost claims, use provider/runtime-reported token accounting only.
- For server isolation claims, report rejection reason and policy evidence, not
  just pass/fail.
- For dynamic loading claims, report cold load, warm load, cache hit/miss, and
  validation time separately.
- Use `lint_skill_claims.py` to reject summaries whose numbers do not trace to
  `results.csv`, diagnostics, metrics, sessions, or trace artifacts.

Minimum `results.csv` columns:

```text
run_id, task_id, benchmark_level, benchmark_family, skill_id, skill_version,
arm, loading_mode, host_agent, model, backend_label, opt_level, iteration,
success, task_score, tests_passed, tests_failed, constraint_violations,
skill_should_activate, skill_did_activate, step_coverage,
wall_ms, graph_duration_ms, compile_wall_ms, load_wall_ms,
llm_call_count, input_tokens, output_tokens, total_tokens, estimated_cost_usd,
nodes_executed, nodes_failed, observed_critical_path_ms,
critical_milestone_last_ms, queue_wait_p95_ms,
fired_passes, total_ops_eliminated, total_tokens_saved,
artifact_hash, session_dir, diagnostics_path, claim_evidence_path
```

For APXM arms, also compute observability coverage from session artifacts:

```text
node_status_coverage = nodes_with_status / node_count
node_output_coverage = output_producing_nodes_with_output / output_producing_nodes
node_trace_coverage = nodes_with_trace / node_count
node_metrics_coverage = nodes_with_metrics / node_count
prompt_coverage = llm_nodes_with_prompt / llm_nodes
response_coverage = llm_nodes_with_response / llm_nodes
skill_metadata_coverage = nodes_with_skill_id / node_count
scope_metadata_coverage = nodes_with_scope_id / node_count
```

Gates:

- A3 and A4 must have complete node status coverage.
- Output-producing nodes must have output evidence unless policy redacts it.
- LLM nodes must have prompt and response evidence unless streaming/output
  capture is explicitly disabled.
- A4 must not reduce observability coverage relative to A3.
- If O2 removes a node, the missing node must be explained by compiler
  diagnostics rather than counted as an observability failure.

Claim linting should reject:

- speed claims based on compile-only rows
- scheduling claims based on wall time when critical-path or milestone metrics
  exist
- token/cost claims without provider/runtime token accounting
- checkpoint/replay claims that only measured HITL pause/resume state
- benchmark-family claims made from a cherry-picked internal subset
- public benchmark claims made against modified harnesses without saying so
- broad "APXM skills improve agents" claims from one skill or one task

## First Experiment: Checkout Context Triage

The first proof should be a hand-authored APXM skill named
`checkout-context-triage`, based on:

- `examples/python/demos/gemma4/workflows/02_checkout_context_pruning.py`
- `examples/python/demos/gemma4/workflows/02_checkout_context_pruning/compiler.md`
- `examples/python/demos/gemma4/workflows/02_checkout_context_pruning/runtime.md`
- `examples/python/demos/gemma4/runs/20260428T053110Z-three-cases-postfix/context-pruning/runtime-o0-o2.csv`

This is the strongest seed because it already demonstrates the exact APXM
compiler value we need to test for skills. O0 keeps five context-extraction LLM
branches plus one decision call. O2 proves four context edges are dead because
the decision template references only `{checkout_context}`, then the cleanup
passes remove the unused upstream LLM nodes.

Existing captured evidence from the three-row run:

| Arm | LLM calls | Total tokens | Compiler evidence |
| --- | ---: | ---: | --- |
| A3 / O0 | 6 per run | about 1,164-1,186 | no ops eliminated |
| A4 / O2 | 2 per run | about 537-552 | `dead-context-elimination`, `scheduling`, `assign-priority`, and `canonicalizer`; 4 ops eliminated |

The first A0-A4 task should be:

> Given the checkout production note, produce one root cause, one hotfix action,
> and one regression test. Do not use API, observability, compliance, or rollout
> notes.

Expected grading assertions:

- The root cause is tied to duplicate `order_authorized` events after provider
  timeout retries.
- The answer mentions the idempotency-key change from
  `cart_id` / `payment_intent_version` to `order_id` / `attempt_no`.
- The hotfix does not require a database migration.
- The regression test covers provider timeout/retry duplication.
- The output does not rely on unrelated API, observability, compliance, or
  rollout facts.

Recommended fixture location:

```text
examples/python/skill-evals/
  README.md
  corpus/
    checkout-context-triage/
      source/
        SKILL.md
      apxm/
        skill.air
        skill.toml
        conversion-report.json
      tasks/
        duplicate-timeout.toml
        negative-api-auth.toml
      gold/
        assertions.py
        expected-output.toml
  scripts/
    run_skill_case.py
    summarize_skill_runs.py
    lint_skill_claims.py
```

Backend-free checks should compile, validate manifests, compare compiler
diagnostics, and run unit assertions. Backend-required checks should run Gemma
or another configured LLM backend and collect O0/O2 call/token/output evidence.

## Metrics And Evidence

Correctness and workflow:

- `task_success`
- tests passed/failed and test runtime
- expected files/artifacts produced
- regressions introduced
- clarification expected/asked
- constraint violations
- unsupported claims

Skill behavior:

- `skill_should_activate`
- `skill_did_activate`
- trigger precision/recall
- step coverage
- missed verification steps
- step order violations
- over-application on negative-trigger tasks

Runtime and optimization:

- wall time, graph duration, compile time, load time
- LLM call count and token usage when provider-reported
- nodes executed, failed, skipped, retried
- observed critical path and critical milestone finish
- queue wait totals and p95
- fired passes, eliminated ops, eliminated calls, tokens saved
- backend cache/pin metrics when applicable

Auditability:

- `manifest.json`, `results.json`, `metrics.json`, `trace.ndjson`
- input AIR, artifact path, compiler diagnostics
- per-node prompt/output/metrics evidence
- replay and decompile success
- source skill hash, AIR hash, artifact hash, fixture hash
- claim evidence coverage

Token and cost claims for spawned Codex or Claude ACP subprocesses must not be
inferred from wall time. Report token/cost only when the ACP adapter or provider
reports usage into APXM token accounting.

## Go/No-Go Gates

Minimum go gates before claiming that APXM-aware skills improve coding-agent
work:

- A3 task success is at least A1 for executable-parity fixtures.
- A4 task success is at least A3; no O2 correctness regression.
- Deterministic fixtures pass at least 4 out of 5 repeated runs, or
  nondeterministic fixtures have documented flake analysis.
- Quality fixtures pass with budget caps.
- Speed claims have repeated rows, meaningful absolute delta, meaningful
  relative delta, and confidence that speedup is greater than 1.0.
- Token/call claims show unchanged success and actual reported token/call
  reduction.
- Runtime scheduling claims use critical-path or milestone metrics under queue
  pressure, not unrelated whole-graph wall time.
- Dynamic loading works cold and warm, with cache hit/miss and load overhead
  measured.
- Every headline claim traces to `results.csv`, session metrics, compiler
  diagnostics, or trace artifacts.

No-go conditions:

- O2 lowers task success relative to O0.
- Host-agent markdown skills beat APXM execution on correctness for fixtures
  meant to test executable parity.
- Cost/token numbers are inferred rather than provider-reported.
- Negative-trigger fixtures show systematic over-activation.
- `trace_diff` shows semantic divergence where O0/O2 should be equivalent.
- Reports contain claims that cannot be traced to artifacts.

## Implementation Phases

### Phase 0: Document And Align

- Land this design note.
- Link it from the docs index.
- Keep server and dynamic loading work out of this phase.

### Phase 1: Read-Only Skill Library

- Add a shared skill-library scanner/model, preferably outside GUI-specific
  code.
- Discover source skills from `.agents/skills` and configured roots.
- Read `SKILL.md`, `skill.toml`, `skill.air`, and artifact metadata when
  present.
- Expose `GET /v1/skills` and `GET /v1/skills/{id}` from `apxm-server`.
- Update GUI to consume the server-owned endpoint instead of owning discovery.

### Phase 2: Validation And Compilation

- Add validation for manifests, AIR, hashes, and capability requirements.
- Add `POST /v1/skills/{id}/validate`.
- Add explicit `POST /v1/skills/{id}/compile` with opt level and target.
- Cache artifacts by content hash and compiler/runtime version.
- Add MCP tools for list/get/validate/compile/explain.

### Phase 3: Conversion

- Implement `skill-to-air` as a report-first converter.
- Emit skeleton AIR only when required structure is identifiable.
- Require conversion reports for any inferred graph structure.
- Add fixtures for source skills, generated AIR, and round-trip validation.

### Phase 4: Execution And Dynamic Loading

- Execute statically compiled APXM skills first.
- Add hybrid loading after compile/cache behavior is stable.
- Add dynamic loading only with explicit capability policy and fail-closed
  safety checks.
- Record loader metrics in sessions.

### Phase 5: Evaluation And Claims

- Create `examples/python/skill-evals/` around the A0-A4 matrix.
- Wrap `benchmark_e2e.py` instead of duplicating runtime measurement.
- Add claim linting so reports cannot cite unsupported numbers.
- Promote claims into demos/decks only after artifact-backed runs exist.

## Test Plan

Backend-free checks:

```bash
python3 examples/python/demos/gemma4/scripts/run_demo.py \
  --run-dir /tmp/checkout-context-triage-demo \
  --backend vllm-fork \
  --model google/gemma-4-31B-it \
  --skip-probe

python -m pytest tools/quality_eval/tests -q
bash tools/eval_harness/tests/test_trace_diff.sh
cargo test -p apxm-artifact --locked
cargo check -p apxm-cli --features driver,metrics --locked
python3 examples/python/benchmarks/benchmark_e2e.py \
  --graph examples/python/demos/gemma4/workflows/02_checkout_context_pruning.py \
  --compile-only \
  --opt-level 0 \
  --opt-level 2 \
  --emit-compiler-diagnostics \
  --diagnostics-dir /tmp/checkout-context-triage/diagnostics \
  --output /tmp/checkout-context-triage/compile-only.csv \
  --apxm-config /tmp/checkout-context-triage-demo/generated/config.toml
```

This is backend-free in the sense that `--skip-probe` avoids contacting vLLM
and `--compile-only` avoids graph execution. The generated config is still
required because the workflow resolves symbolic `showcase` and `benchmark`
route aliases while emitting AIR.

Backend-required A3/A4 checks:

```bash
python3 examples/python/benchmarks/benchmark_e2e.py \
  --graph examples/python/demos/gemma4/workflows/02_checkout_context_pruning.py \
  --iterations 3 \
  --precompile-artifacts \
  --emit-compiler-diagnostics \
  --artifact-dir "$RUN_DIR/checkout-context-triage/artifacts" \
  --diagnostics-dir "$RUN_DIR/checkout-context-triage/compiler-diagnostics" \
  --output "$RUN_DIR/checkout-context-triage/runtime-o0-o2.csv" \
  --session-base "$RUN_DIR/checkout-context-triage/sessions" \
  --backend-label skill-eval-vllm \
  --target tokens \
  --apxm-config "$CFG" \
  --interleave-opt-levels
```

The host-agent A0/A1/A2 runner is not present yet. The first implementation of
`run_skill_case.py` should create deterministic task bundles and record host
outputs, but it must not infer host token or cost usage unless the host adapter
reports it.

## Non-Goals For The First Cut

- Do not rewrite Codex.
- Do not make dynamic execution the first milestone.
- Do not treat generated AIR as correct because it compiles.
- Do not duplicate large docs into every skill package.
- Do not make the GUI the source of truth for skill discovery.
- Do not infer ACP subprocess costs or tokens from elapsed time.
- Do not scan or import `external/vllm` as an ordinary APXM skill source.

## Open Questions

- What is the stable APXM skill ABI beyond `inputs`, `outputs`,
  `capabilities`, `resources`, `side_effects`, and `approval_policy`?
- Should skill-library state live in `apxm-driver`, `apxm-runtime`, a new
  orchestration crate, or `apxm-server` first and then be factored out?
- How should APXM represent intentionally judgment-heavy steps: PLAN nodes,
  autonomous zones, or host-agent callbacks behind typed boundaries?
- Are scripts inside source skills runtime capabilities, conversion-time
  helpers, or both?
- What safety policy applies to dynamically loaded skills with shell scripts?
- How should remote/marketplace skill trust be represented in manifests?

## Immediate Next Steps

1. **Execution retention:** add a bounded or reloadable execution index over the
   persisted `executions/{execution_id}.json` session snapshots.
2. **G2 remainder:** nested parent provenance and artifact-derived node names
   for redacted `llm_prompt`, `node_output`, and `node_metrics` events.
3. **Capability policy remainder:** extend the current read-only registered
   capability admission with sandbox routing and tests for side-effectful
   policies.
4. **G3-G5 / Nested observability:** fix child artifact session metadata,
   namespace `FLOW_CALL` child outputs, and persist scope ids in session/event
   metadata.
5. **G6 / Real checkpointing:** define scheduler snapshots and unify
   `CHECKPOINT`, `PAUSE`, and `RESUME` around that snapshot model before making
   replay/checkpoint claims.
6. **Evaluation fixture:** create `examples/python/skill-evals/` only after the
   provenance and event surfaces exist, then run the A0-A4 checkout-context
   fixture and server-isolation benchmarks.
