# APXM Skill Runtime Task Backlog

This backlog turns the APXM-aware skill-library design into PR-sized work. The
goal is to make APXM a server of validated, isolated, observable compiled skills
that Codex, Claude Code, MCP clients, the GUI, and other agents can call without
opening unsafe arbitrary execution.

This document is intentionally task-oriented. The narrative design remains in
`docs/design/apxm-aware-codex-skill-libraries.md`.

## Current Implemented Slice

The first implementation slice now exists in `apxm-server`:

- configured server skill roots through repeated `--skill-root <path>` and
  `APXM_SKILL_ROOTS`;
- `SkillManifest` parsing and `SkillLibrary` scanning for `skill.toml`,
  `SKILL.md`, `skill.air`, and `skill.apxmobj`;
- server-owned inventory routes:

```text
GET /v1/skills
GET /v1/skills/{id}
POST /v1/skills/{id}/validate
```

- narrow static execution through `POST /v1/skills/{id}/execute`;
- streaming static execution through `POST /v1/skills/{id}/execute/stream`;
- in-memory skill execution records through `GET /v1/executions/{execution_id}`;
- in-memory recorded node outputs through
  `GET /v1/executions/{execution_id}/nodes/{node_id}`;
- MCP tools `apxm_skills_list`, `apxm_skill_get`,
  `apxm_skill_validate`, and `apxm_skill_call`.

The static execution path only accepts manifest-identified, precompiled
artifacts from server-owned skill roots. It rejects raw AIR, client-provided
artifact paths, client-provided session roots, symlinked artifacts, missing or
mismatched artifact hashes, entry-flow mismatches, invalid session ids, and
unsafe operations. Static `INV_TOOL` nodes are admitted only when the manifest
declares the capability/tool, the capability is registered in the runtime, the
capability metadata is read-only, the manifest side-effect policy is omitted or
`read_only`, and the artifact does not use Python-backed tool handlers.
Streaming skill execution now forwards typed `node_output` and `node_metrics`
core events through `EmitterAdapter`, and REST/SSE skill execution records
node outputs for node-detail lookup. Prompt observability now emits redacted
`llm_prompt` events, and `node_output` events carry summary/hash/redaction
metadata instead of raw JSON values. Execution records are memory-indexed for
API lookup and snapshotted to `executions/{execution_id}.json` inside the
APXM-owned skill session directory. Runtime events emitted by server-owned skill runs carry
`skill_id`, `skill_version`, and entry-flow provenance. Nested parent-skill
provenance, artifact-level provenance, and full scheduler replay remain future
work.

## Next PR

The next engineering PR should move from static execution to observable
server-managed executions:

1. Add a bounded or reloadable execution index on top of persisted
   `executions/{execution_id}.json` snapshots.
2. Preserve parent-run provenance, `scope_id`, and artifact/session provenance
   for nested skills.
3. Extend capability admission beyond read-only registered tools with explicit
   sandbox preflight.

## Operating Principles

- Start with read-only discovery and validation. Do not execute skills until
  manifest identity, hash checks, capability policy, and server boundaries exist.
- Execute static, precompiled `.apxmobj` skills before dynamic conversion or
  dynamic compilation.
- Keep `/v1/execute` as a developer raw-AIR endpoint. Agent-facing clients must
  call skills by manifest identity.
- Make top-level skill runs observable before claiming nested skill-library
  observability.
- Do not claim checkpoint/replay until APXM has scheduler snapshots, not only
  HITL pause/resume state.
- Every benchmark or marketing claim must trace to artifacts: CSV rows, session
  metrics, compiler diagnostics, event traces, or test output.

## Priority Map

| Priority | Track | Outcome |
| --- | --- | --- |
| P0 | Manifest and read-only registry | APXM can list and validate installed skills safely. |
| P1 | Static skill execution | APXM can run precompiled, hash-validated skill artifacts with server-owned sessions. |
| P1 | Typed observability | Streams and sessions expose node outputs, prompts, metrics, skill identity, and scopes. |
| P2 | Nested skill execution | Parent graphs can call child artifacts/skills and keep child evidence inspectable. |
| P2 | Evaluation harness | A0-A4 benchmarks produce defensible rows and claim reports. |
| P3 | Dynamic loading/conversion | APXM can convert/compile/cache skills safely on demand. |
| P3 | Scheduler checkpoint/replay | Skill runs can pause, checkpoint, resume, and replay from scheduler state. |

## Gap Closure Matrix

| Missing piece | Closing tasks | Done when |
| --- | --- | --- |
| Server-owned top-level runtime events carry `skill_id`, `skill_version`, and entry flow; artifact/session metadata and nested parent provenance remain incomplete. | T0.1, T2.3, T3.3 | Manifests, artifacts, sessions, events, and nested invocations preserve skill identity and parent links. |
| Generic event streams include typed redacted `llm_prompt`, `node_output`, and `node_metrics`, but still need node names and nested parent scope/provenance. | T2.1, T2.2, T2.3 | REST/MCP streaming consumers can connect runtime events to node names, scope, and parent run. |
| Child artifact workflow sessions can miss per-node directories. | T3.1 | Child artifact executions reconstruct graph metadata and write complete per-node evidence. |
| `FLOW_CALL` returns only the sub-flow result and hides child output maps. | T3.2 | Parent results and sessions expose namespaced child `all_outputs` / `node_output_map` data. |
| Session scope ids exist in lower layers but are not persisted as first-class isolation dimensions. | T2.3, T3.3 | Session manifests, node files, event streams, and skill/scope indices include non-null scope ids for skill runs. |
| `CHECKPOINT`, `PAUSE`, and `RESUME` are not full scheduler-state snapshots. | T6.1, T6.2, T6.3 | Snapshots include scheduler queues, token values, node outputs, op states, graph/session/skill metadata, and safe backend resume state. |
| Skill execution exists only as a narrow static endpoint. | T1.2, T1.3, T2.1, T2.2 | `apxm-server` records skill executions, streams typed events, and exposes node outputs without raw AIR or client paths. |
| Evaluation claims need reproducible evidence. | T4.1, T4.2, T4.3 | A0-A4 rows, diagnostics, sessions, metrics, and claim-linter output exist for each claim. |

## P0: Read-Only Skill Foundation

### T0.0 Configure Server Skill Roots

**Why:** `SkillLibrary` cannot be deterministic until `apxm-server` has an
explicit source of installed skill roots.

**Scope:**

- Add a server configuration surface for skill roots, such as repeated
  `--skill-root` flags plus an `APXM_SKILL_ROOTS` fallback.
- Use a deterministic default only when safe, for example workspace
  `.agents/skills` and an APXM-owned server skill directory.
- Reject or ignore client-provided paths on read-only REST requests.
- Record configured roots in server startup logs and test state.

**Likely files:**

- `crates/tools/apxm-server/src/main.rs`
- `crates/tools/apxm-server/src/state.rs`
- `crates/tools/apxm-server/src/skills.rs`

**Acceptance criteria:**

- Tests can start `apxm-server` with fixture skill roots.
- `AppState` owns a `SkillLibrary` initialized from configured roots.
- `GET /v1/skills` never accepts a root/path query parameter.
- Missing roots are reported without crashing the server.

### T0.1 Define `SkillManifest` And `SkillProvenance`

**Why:** Every later feature needs stable identity: skill id, version, artifact
hash, entry flow, capability policy, and ABI.

**Scope:**

- Define a minimal manifest model.
- Support `skill.toml` and, later, embedded artifact section
  `apxm.skill_manifest.v1`.
- Include `skill_id`, `version`, optional `display_name`, `description`,
  `entry_flow`, source hash, AIR hash, artifact hash, compiler/runtime
  compatibility, input/output ABI, required capabilities, allowed tools,
  timeout, token limit, isolation policy, and side-effect policy.

**Likely files:**

- New shared module or crate: `crates/orchestration/apxm-skill`
- If starting smaller: `crates/tools/apxm-server/src/skills.rs`
- Later consumers: `crates/orchestration/apxm-driver/src/session_output.rs`
- Artifact section constants may live near `crates/orchestration/apxm-artifact/src/lib.rs`

**Acceptance criteria:**

- Valid `skill.toml` parses into a typed manifest.
- Missing `skill_id`, `version`, or `entry_flow` fails clearly.
- Invalid timeout/token/isolation policy fails validation.
- Manifest can be serialized as JSON for server responses.

**Tests:**

- Unit tests for valid manifest parsing.
- Unit tests for missing required fields.
- Unit tests for malformed policy and ABI.

### T0.2 Add `SkillLibrary` Scanner

**Why:** The immediate next engineering PR should be a read-only
`SkillManifest` + `SkillLibrary` scanner exposed by `apxm-server`.

**Scope:**

- Scan configured skill roots.
- Recognize packages containing `skill.toml`.
- Optionally detect `SKILL.md`, `skill.air`, `skill.apxmobj`,
  `conversion-report.json`, `resources/`, and `tests/`.
- Compute source/AIR/artifact hashes.
- Return validation status without executing anything.
- Do not follow untrusted roots supplied through REST query parameters.

**Likely files:**

- `crates/tools/apxm-server/src/skills.rs`
- `crates/tools/apxm-server/src/state.rs`
- Later extraction to `crates/orchestration/apxm-skill`

**Acceptance criteria:**

- Scanner returns stable `SkillRecord` entries.
- Duplicate skill id/version is rejected or reported deterministically.
- Artifact hash mismatch is reported as validation failure.
- Missing artifact is allowed for source-only or compilable skills but marked
  `not_compiled`.

**Tests:**

- Fixture skill root with one valid package.
- Fixture with duplicate id/version.
- Fixture with bad artifact hash.
- Fixture with source-only skill.

### T0.3 Expose Read-Only Skill Server Endpoints

**Why:** This creates the foundation without opening unsafe execution.

**Server API:**

```text
GET  /v1/skills
GET  /v1/skills/{id}
POST /v1/skills/{id}/validate
```

**Scope:**

- Add server routes for listing and inspecting skills.
- Add `SkillLibrary` to `AppState`.
- Return manifest, validation status, paths/hashes, ABI summary, capability
  requirements, policy summary, and compile status.
- Do not run, compile, or load arbitrary client-provided paths.
- Do not expose filesystem roots or absolute package paths unless the server is
  running in an explicit developer/debug mode.

**Likely files:**

- `crates/tools/apxm-server/src/main.rs`
- `crates/tools/apxm-server/src/state.rs`
- `crates/tools/apxm-server/src/skills.rs`
- `crates/tools/apxm-server/src/tests.rs`

**Acceptance criteria:**

- `GET /v1/skills` returns installed skill records.
- `GET /v1/skills/{id}` returns one manifest and status.
- `POST /v1/skills/{id}/validate` reruns validation and returns structured
  failures.
- Unknown skill returns 404.

**Tests:**

```bash
cargo test -p apxm-server skills_list_returns_installed_manifests
cargo test -p apxm-server skill_detail_returns_manifest_and_status
cargo test -p apxm-server skill_validate_reports_hash_mismatch
```

### T0.4 Wire GUI To Server-Owned Skill Inventory

**Why:** The GUI skill endpoint is currently a placeholder. The server should be
the source of truth.

**Scope:**

- Replace empty GUI skill response with server/library-backed data.
- Show source-only, compilable, compiled, invalid, and disabled status.
- Surface validation errors and artifact hashes.

**Likely files:**

- `crates/tools/apxm-gui/src/api/skills.rs`
- `crates/tools/apxm-gui/frontend/src/types/api.ts`
- GUI views that consume skill data

**Acceptance criteria:**

- GUI returns non-empty skills when a fixture skill root is configured.
- Invalid skills show validation failures without crashing.

## P1: Static Skill Execution

### T1.1 Harden Server Runtime Initialization

**Why:** Bare `Runtime::new(RuntimeConfig::default())` does not configure the
same LLM registry, capability registry, sandbox registry, model router,
middlewares, agent registry, or workflow spawner as the driver.

**Scope:**

- Extract or reuse the driver runtime setup path for `apxm-server`.
- If direct reuse would create a bad dependency direction, introduce a shared
  runtime-builder module/crate consumed by both driver and server.
- Update Cargo dependencies explicitly instead of duplicating driver-private
  setup code in the server.
- Ensure skill execution uses the configured sandbox registry and capability
  system.
- Keep raw developer `/v1/execute` behavior separate from skill-server policy.

**Likely files:**

- `crates/orchestration/apxm-driver/src/runtime/mod.rs`
- `crates/tools/apxm-server/src/main.rs`
- `crates/tools/apxm-server/src/state.rs`
- `crates/tools/apxm-server/Cargo.toml`
- possibly a shared runtime-builder module/crate

**Acceptance criteria:**

- Server runtime has configured capabilities and sandbox registry.
- Server runtime has the same model router policies, ACP agent registry,
  middlewares, inner-plan linker, and workflow spawner expected by driver-backed
  execution when those features are enabled.
- Skill mode can deny missing capabilities before execution.
- Server tests can inject a test runtime and test skill library.

### T1.2 Add Static Skill Execution Endpoints

**Server API:**

```text
POST /v1/skills/{id}/execute
POST /v1/skills/{id}/execute/stream
GET  /v1/executions/{execution_id}
GET  /v1/executions/{execution_id}/nodes/{node_id}
```

**Scope:**

- Execute only precompiled, hash-validated `.apxmobj` artifacts.
- Add an `ExecutionStore` or run registry for skill executions.
- Validate JSON/object arguments against the skill ABI.
- Validate entry DAG and positional/runtime args.
- Require timeout and token limits.
- Create APXM-owned session directories; do not accept arbitrary
  client-provided `session_root`.
- Enable `SchedulerConfig.collect_all_outputs` when node inspection is
  promised.
- Return execution id, status, result summary, stats, session path, and node
  summary.
- Persist lifecycle state, retention policy, and node-summary indices for
  `GET /v1/executions/{execution_id}` and node inspection.

**Likely files:**

- `crates/tools/apxm-server/src/skills.rs`
- `crates/tools/apxm-server/src/state.rs`
- `crates/tools/apxm-server/src/execute.rs` for shared response helpers
- possibly `crates/tools/apxm-server/src/executions.rs`
- `crates/runtime/apxm-runtime/src/runtime.rs`

**Acceptance criteria:**

- Valid pure skill artifact executes successfully.
- Bad artifact hash rejects before runtime execution.
- Missing required capability rejects before runtime execution.
- Client cannot choose a session root outside APXM-owned storage.
- Streaming endpoint emits lifecycle events and final result.
- Execution detail endpoint returns status, session path, stats, and node
  summaries for completed and failed runs.
- Node detail endpoint resolves from the execution store/session index rather
  than scanning arbitrary client-supplied paths.

**Tests:**

```bash
cargo test -p apxm-server skill_execute_static_artifact_returns_result
cargo test -p apxm-server skill_execute_rejects_artifact_hash_mismatch
cargo test -p apxm-server skill_execute_rejects_required_capabilities_until_policy_exists
cargo test -p apxm-server skill_execute_rejects_client_session_root_field
cargo test -p apxm-server skill_execute_stream_emits_final_result
cargo test -p apxm-server mcp_skill_call_executes_static_server_owned_skill
```

### T1.3 Add Generic MCP Skill Tools

**Why:** Codex, Claude Code, and other clients should call one policy-checked
skill API first, not a dynamic tool for every skill.

**MCP tools:**

```text
apxm_skills_list
apxm_skill_get
apxm_skill_validate
apxm_skill_call
apxm_skill_run_status  # future, after ExecutionStore lands
```

**Scope:**

- Add MCP methods backed by the server skill library.
- Keep generic `apxm_skill_call` rather than exposing every skill as an MCP
  tool.
- Enforce the same manifest, capability, timeout, token, and sandbox policy as
  REST.

**Likely files:**

- `crates/tools/apxm-server/src/mcp.rs`
- `crates/tools/apxm-server/src/bin/apxm_mcp.rs`
- `crates/tools/apxm-server/src/skills.rs`

**Acceptance criteria:**

- MCP can list skills.
- MCP can get one skill manifest.
- MCP `apxm_skill_call` executes a valid static skill.
- MCP cannot bypass capability or sandbox policy.

## P1: Typed Observability

### T2.1 Add Core Node Event Payloads

**Why:** `SessionEventEmitter` writes files, and generic event streams now
include redacted `llm_prompt`, `node_output`, and `node_metrics` payloads.
Top-level server-owned skill runs attach `skill_id`, `skill_version`, and entry
flow to those runtime events. The remaining work is enriching those payloads
with node names, nested parent provenance, and session-level scope persistence.

**Scope:**

- Done: add `node_output` event kind and payload, with `node_id` plus redacted
  summary/hash metadata.
- Done: add `node_metrics` event kind and payload, with `node_id` and
  provider-neutral node metrics.
- Done: add event kind and payload struct for redacted `llm_prompt`.
- Extend node payloads with optional `node_name`, `skill_id`, and `flow_name`.
  `scope_id` already travels in the event envelope when the runtime sets it.

**Likely files:**

- `crates/core/apxm-core/src/events/kind.rs`
- `crates/core/apxm-core/src/events/payload.rs`
- `crates/runtime/apxm-runtime/src/executor/events.rs`

**Acceptance criteria:**

- `node_output` payloads serialize/deserialize through `ApxmEvent`.
- `node_metrics` payloads serialize/deserialize through `ApxmEvent`.
- Redacted `llm_prompt` payloads serialize/deserialize through `ApxmEvent`.
- Redacted payloads do not include full values.

### T2.2 Forward Node Events From `EmitterAdapter`

**Scope:**

- Done: implement `emit_node_output` in `EmitterAdapter`.
- Done: implement `emit_node_metrics` in `EmitterAdapter`.
- Done: implement `emit_llm_prompt` in `EmitterAdapter`.
- Preserve span and scope metadata.
- Keep event order coherent with operation lifecycle.

**Likely files:**

- `crates/runtime/apxm-runtime/src/executor/emitter_adapter.rs`
- `crates/runtime/apxm-runtime/tests/span_hierarchy.rs`
- server stream tests

**Acceptance criteria:**

- Streamed runtime execution can observe node-output and node-metrics events.
- LLM prompt payloads are visible to event consumers with redaction policy.
- Existing session file behavior is unchanged.
- Root and per-node `trace.ndjson` files include typed `node_output`,
  `node_metrics`, and redacted `llm_prompt` events when those payloads are
  emitted.

**Tests:**

```bash
cargo test -p apxm-runtime emitter_adapter_forwards_node_output_events
cargo test -p apxm-runtime emitter_adapter_forwards_node_metrics_events
cargo test -p apxm-runtime emitter_adapter_forwards_redacted_llm_prompt_events
cargo test -p apxm-server skill_execute_stream_emits_node_output_events
cargo test -p apxm-driver session_trace_contains_typed_node_events
```

The first three tests exist for the runtime/server event stream; the
driver/session trace test belongs with prompt redaction and persistent session
trace work.

### T2.3 Persist Skill And Scope Metadata In Sessions

**Why:** Skill runs need a first-class isolation dimension, not just node ids.

**Scope:**

- Implement `set_current_scope_id` and `current_scope_id` for
  `SessionEventEmitter`.
- Make `SessionOutputWriter::write_manifest` and node metadata writing accept
  the active scope instead of hard-coding `None`.
- Stamp scope id into root events, node events, session manifest, node metadata,
  and skill/scope indices.
- Add `skills/<skill_run_id>/` or `scopes/<scope_id>/` index files.

**Likely files:**

- `crates/orchestration/apxm-driver/src/session_output.rs`
- `crates/runtime/apxm-runtime/src/executor/context.rs`
- `crates/core/apxm-core/src/events/mod.rs`

**Acceptance criteria:**

- Scoped child executions write non-null scope ids.
- Session manifests and node metadata persist the active scope id.
- Concurrent runs of the same skill do not mix outputs.
- Session inspection can list all nodes for one skill invocation.

## P2: Nested Skill Execution And Evidence

### T3.1 Fix Child Artifact Workflow Sessions

**Why:** Child artifact workflow sessions can miss per-node directories because
the driver does not pass reconstructed artifact graph metadata into the child
session emitter.

**Scope:**

- Reconstruct a graph metadata view from a child artifact's entry DAG or DAG
  list.
- Pass that metadata into child session writer/emitter.
- Write child artifact hash, parent execution id, and parent node id.
- Extend the workflow invocation or runtime spawn context to carry parent
  execution/node links when needed.

**Likely files:**

- `crates/orchestration/apxm-driver/src/runtime/workflow_spawn.rs`
- `crates/orchestration/apxm-driver/src/session_output.rs`
- `crates/runtime/apxm-runtime/src/executor/handlers/workflow_spawn.rs`
- `crates/core/apxm-core/src/types/execution/workflow.rs`
- `crates/tools/apxm-cli/src/commands/execute.rs`

**Acceptance criteria:**

- Parent `WORKFLOW_SPAWN artifact_path` creates a child session.
- Child session has per-node directories, output files, statuses, and traces.
- Parent session records the child session path and artifact hash.

### T3.2 Namespace `FLOW_CALL` Child Outputs

**Why:** `FLOW_CALL` currently returns the sub-flow result but does not expose
child `all_outputs` / `node_output_map` under a skill namespace.

**Scope:**

- Propagate runtime scheduler config into child flow calls.
- Collect child output maps when parent output collection is enabled.
- Define the result contract for child outputs, either with a structured
  `child_outputs` field or a typed output-key model. Do not try to force
  string namespaces into current `u64` node-output maps.
- Store child outputs under `flow::<agent>.<flow>` or
  `skill::<skill_run_id>/<flow>`.
- Emit child output summaries through typed node-output events.

**Likely files:**

- `crates/runtime/apxm-runtime/src/executor/handlers/flow_call.rs`
- `crates/runtime/apxm-runtime/src/scheduler/dataflow.rs`
- `crates/runtime/apxm-runtime/src/executor/hooks.rs`
- `crates/runtime/apxm-runtime/src/runtime.rs`

**Acceptance criteria:**

- Parent graph calls a registered flow with multiple child outputs.
- Runtime result exposes parent output and namespaced child outputs.
- `RuntimeExecutionResult` or an adjacent result type has an explicit contract
  for child output maps.
- Session output distinguishes parent node outputs from child flow outputs.

### T3.3 Add Named Skill Invocation

**Candidate APIs:**

```text
WORKFLOW_SPAWN(target_kind = "skill", target = "checkout-context-triage@0.1.0")
INV_TOOL(capability = "skill:checkout-context-triage")
```

**Scope:**

- Add a `Skill` workflow target.
- Resolve skill id/version through `SkillLibrary`.
- Execute through static artifact path first.
- Preserve parent/child skill provenance and scope.

**Likely files:**

- `crates/core/apxm-core/src/types/execution/workflow.rs`
- `crates/runtime/apxm-runtime/src/executor/handlers/workflow_spawn.rs`
- `crates/orchestration/apxm-driver/src/runtime/workflow_spawn.rs`
- `crates/compiler/apxm-frontend/python/apxm/proxy.py`
- `crates/compiler/apxm-compiler/mlir/lib/Dialect/AIS/IR/AISOps.cpp`

**Acceptance criteria:**

- Parent graph invokes a named skill.
- Child nodes appear under a distinct skill invocation.
- Unknown/disabled skill fails before runtime execution.

## P2: Evaluation And Benchmarks

### T4.1 Create `examples/python/skill-evals/`

**Scope:**

```text
examples/python/skill-evals/
  README.md
  corpus/
    checkout-context-triage/
      source/SKILL.md
      apxm/skill.toml
      apxm/skill.air
      apxm/conversion-report.json
      tasks/
      gold/
  scripts/
    run_skill_case.py
    summarize_skill_runs.py
    lint_skill_claims.py
```

**Acceptance criteria:**

- Fixture validates without a backend.
- A3/A4 can run through precompiled artifacts.
- Results rows include skill id/version, arm, opt level, artifact hash,
  session path, diagnostics path, quality result, and observability coverage.

### T4.2 Implement A0-A4 Runner

**Scope:**

- A0: host agent or host proxy receives only task.
- A1: host receives markdown `SKILL.md`.
- A2: host receives readable `skill.air`.
- A3: APXM executes O0 static artifact.
- A4: APXM executes O2 static artifact.
- Interleave arms by block.
- Normalize prompt budgets, tool permissions, timeout, model/backend, and input
  files across A0/A1/A2 host-agent arms.
- Score A0/A1/A2 through `tools/quality_eval` where possible instead of manual
  pass/fail notes.
- Do not infer host token/cost unless adapter reports it.

**Likely files:**

- `examples/python/skill-evals/scripts/run_skill_case.py`
- `examples/python/skill-evals/scripts/summarize_skill_runs.py`
- `tools/quality_eval/runner.py`
- `tools/quality_eval/tests/`

**Acceptance criteria:**

- Runner emits one CSV row per `(task, arm, iteration)`.
- Rows use the shared result schema: run id, task id, benchmark level, arm,
  skill id/version, model/backend, prompt budget, timeout, token metrics when
  reported, quality result, artifact/session paths, and diagnostics paths.
- A0/A1/A2 rows include captured host prompts/outputs or an explicit redacted
  artifact path.
- A3/A4 rows include compiler diagnostics and session paths.
- A4 cannot be considered a win unless quality is non-regressed.

### T4.3 Add Claim Linter

**Scope:**

- Reject unsupported speed, token, cost, checkpoint, isolation, and broad
  benchmark claims.
- Require all numbers to trace to CSV, diagnostics, metrics, sessions, or trace
  artifacts.

**Acceptance criteria:**

- Claim about speed fails on compile-only rows.
- Claim about cost fails without token accounting and price assumptions.
- Claim about checkpoint/replay fails unless scheduler snapshot evidence exists.

## P3: Dynamic Loading And Conversion

### T5.1 Add Skill Compile Endpoint

**Server API:**

```text
POST /v1/skills/{id}/compile
```

**Scope:**

- Admin/dev only.
- Compile `skill.air` to `.apxmobj` with explicit opt level and target.
- Cache by source hash, compiler version, runtime compatibility, opt level, and
  target.
- Attach or validate manifest section.

**Acceptance criteria:**

- Cold compile produces artifact and diagnostics.
- Warm compile hits cache.
- Compile endpoint is unavailable in locked-down skill-server mode.

### T5.2 Implement Report-First `skill-to-air`

**Scope:**

- Parse `SKILL.md` frontmatter and markdown.
- Emit normalized `SkillSpec`.
- Emit skeleton `skill.air` only when structure is identifiable.
- Produce `conversion-report.json` with unmapped sections, assumptions,
  inferred decisions, missing capabilities, and risk flags.

**Acceptance criteria:**

- Converter fails closed on ambiguous production mappings.
- Generated AIR validates when emitted.
- Conversion report identifies every inferred or unmapped part.

### T5.3 Hybrid/Dynamic Loading

**Scope:**

- Static metadata discovery.
- Compile/cache on first use.
- Fail closed on invalid manifests, missing capabilities, unsafe scripts, or
  disallowed tools.

**Acceptance criteria:**

- Cold and warm load times are measured.
- Cache hit/miss is recorded in session metrics.
- Dynamic loading can be disabled globally.

## P3: Scheduler Checkpoint And Replay

### T6.1 Define Scheduler Snapshot

**Scope:**

- Snapshot token values, node outputs, op states, ready/running/completed
  queues, retry state, delegated/promise tokens, graph/session/skill metadata,
  AAM/memory references, and side-effect safety markers.

**Likely files:**

- `crates/runtime/apxm-runtime/src/scheduler/state.rs`
- `crates/runtime/apxm-runtime/src/scheduler/dataflow.rs`
- `crates/runtime/apxm-runtime/src/executor/handlers/checkpoint.rs`

**Acceptance criteria:**

- Snapshot can distinguish completed, ready, running, and pending work.
- Snapshot records output index and skill/scope metadata.
- Snapshot format is versioned.

### T6.2 Unify `CHECKPOINT`, `PAUSE`, And `RESUME`

**Scope:**

- Make all three use the scheduler snapshot model.
- Emit checkpoint saved/restored events with scope and skill metadata.
- Reject duplicate or unsafe side-effect replay.

**Acceptance criteria:**

- Checkpoint after node N, resume, and match uninterrupted final output.
- Non-idempotent side-effect nodes are not rerun silently.
- Missing backend/capability state gives clear resume errors.

### T6.3 Replay Modes

**Modes:**

```text
inspect              # no execution; inspect node outputs and checkpoints
resume               # continue from checkpoint
replay-deterministic # reuse recorded node outputs
replay-live          # rerun from checkpoint and compare output hashes
```

**Acceptance criteria:**

- Completed skill run can be inspected by `skill_run_id`.
- Paused skill run can resume from snapshot.
- Replay report records skipped/rerun node sets and output hash matches.

## Server Isolation Test Backlog

These tests prove APXM can safely act as an isolated skill server.

```bash
cargo test -p apxm-server server_execute_inv_uses_configured_sandbox_backend
cargo test -p apxm-server server_execute_inv_rejects_missing_sandbox_backend
cargo test -p apxm-server mcp_tools_call_sandboxed_capability
cargo test -p apxm-server server_policy_denies_disallowed_bash_before_sandbox
cargo test -p apxm-server server_execute_stream_emits_ordered_runtime_events_and_execute_complete
cargo test -p apxm-server server_same_session_serializes_different_sessions_parallel
cargo test -p apxm-server task_queue_concurrent_claims_are_unique
cargo test -p apxm-driver --test sandbox_e2e sandbox_backend_strips_sensitive_env_overrides
cargo test -p apxm-driver --test sandbox_e2e bubblewrap_denies_write_outside_declared_paths
```

Do not count the process fallback as OS isolation. It is policy-only and cannot
enforce filesystem or network isolation.

## Benchmark Backlog

### Internal Ladder

1. `tests/quality_fixtures/optimisation_invariant`: deterministic zero-token
   smoke for O0/O2 no-regression.
2. `tests/quality_fixtures/qa_factual`, `no_hallucination`,
   `code_completion`, `summarize_short`: existing quality contracts.
3. `examples/python/benchmarks/stress/*`: compiler microbenchmarks for DCE,
   CSE, prefix hints, memoization, priority, and sequential chains.
4. `examples/python/skill-evals/corpus/checkout-context-triage`: first A0-A4
   skill fixture.
5. Server-isolation tests: skill-server trust boundaries and multi-client
   behavior.
6. Nested-skill observability tests: child artifacts, named skills, flow calls,
   scopes.

### External Ladder

1. Terminal-Bench-style subsets: terminal orchestration, sandboxing, sessions,
   checkpoints.
2. Tau-bench/tau3-style text domains: policy, tools, stateful interactions.
3. SWE-bench-style coding repair: coding-agent process value; pin exact variant
   and avoid broad claims from adapted subsets.
4. GAIA-style tasks: web/file/tool research after evidence capture matures.
5. OSWorld-style tasks: GUI/computer-use supervision after APXM has a GUI
   adapter.

## Definition Of Done For The First Public Claim

APXM can claim "static compiled skills are discoverable, validated, executable,
and observable" only when:

- `GET /v1/skills` and `GET /v1/skills/{id}` work from server-owned registry.
- A static hash-validated skill artifact runs through `/v1/skills/{id}/execute`.
- The server refuses raw AIR on the skill endpoint.
- The server creates APXM-owned session output.
- Node status and output evidence are complete for output-producing nodes.
- Typed event streams include node outputs or redacted summaries.
- Missing capabilities and hash mismatches fail before runtime execution.
- Benchmark report includes A0-A4 rows or explicitly scopes the claim to A3/A4.
- Claim linter passes.
