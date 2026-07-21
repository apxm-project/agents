# Gao

Gao is the TypeScript reference APXM agent package for designing, reviewing,
and controlling APXM workflows. `agent.toml` is the authored package manifest;
the runtime loads its capabilities, permission policies, prompts, hooks,
and generated integrity seal from this directory.

## Typed capability surface

Every capability has a definition and a permission policy. Builtins resolve
through the runtime registry. TypeScript handlers publish closed JSON argument
schemas in `capabilities/handlers/tools.json`.

| Capability | Binding | Decision | Purpose |
| --- | --- | --- | --- |
| `capability_discovery` | builtin | allow | Discover typed templates without granting authority. |
| `http_get` | builtin | allow | Fetch public HTTP content. |
| `read` | builtin | allow | Read within declared roots. |
| `search_skills` | builtin | allow | Search host-granted Agent Skill metadata. |
| `explain_permission` | TypeScript | allow | Explain a capability's approval posture. |
| `list_files` | TypeScript | allow | List package-local files. |
| `list_local_skills` | builtin | allow | List host-granted Agent Skill metadata. |
| `plan_workflow` | TypeScript | allow | Produce a structured workflow plan. |
| `prepare_validation` | TypeScript | allow | Prepare an authoritative validation request. |
| `read_local_skill` | builtin | allow | Read one host-granted Agent Skill resource. |
| `bash` | builtin | ask | Run a sandboxed command. |
| `compose_workflow` | builtin | ask | Validate and stage a workflow. |
| `run_workflow` | builtin | ask | Run a staged workflow. |
| `write` | builtin | ask | Write within declared roots. |

These flat IDs are the complete public surface. Gao has no package-prefixed,
compatibility, or legacy capability aliases.

## Runtime lifecycle

Gao waits for session input under the `gao` session prefix. Sealed package
prompts supply its instruction layer; host and hook values remain typed data and
cannot modify that layer. A successful turn follows this order:

1. Model calls may invoke capabilities. `pre_cap` gates `compose_workflow`, and
   `post_cap` redacts sensitive result markers.
2. `post_turn` compacts long conversations into `gao:conversation:summary`.
3. The input wait re-arms for the next request.

Selected Agent Skills are resolved by the Server catalogue and sealed into
context. The host owns catalogue visibility, resource confinement, digest and
size checks, quotas, and audit; Gao declares only the standard capability IDs
and allowed selection policy. Executable authority stays in package
capabilities and workflow/program metadata.

## Public observability

Gao execution is observable through `apxm.event.v1`:

- `turn_boundary` emits a one-based `turn_number` with `direction = "request"`
  before turn hooks and a matching `direction = "response"` only after the
  successful response hooks, memory writes, and compaction path. A failed turn
  does not emit a false response boundary.
- `llm_step_completed` emits after every model call, including tool-loop steps
  and memoized calls. It carries the node id, one-based step number, model,
  finish reason, detailed token usage, timing, and tool-call count. It is
  non-terminal.
- `llm_done` emits the final model response for a turn with content, model,
  finish reason, usage, and any model-requested tool calls. It is atomic but
  non-terminal for the re-armed session, so observers continue through the
  response boundary and later turns.
- Tool activity is visible as the model's `tool_call`, runtime
  `tool_start`/`tool_end`, and agent-scoped `tool_call_begin`/`tool_call_end`
  events. Agent-scoped events expose argument/result keys, status, and latency
  without copying values into the public payload.
- Every event carries `trace_id` and `span_id`, plus `parent_span_id` and
  `scope_id` when present. The OpenTelemetry exporter preserves this lineage
  and exports model, finish-reason, usage, step, turn, timing, and tool
  attributes on `apxm.<event-kind>` spans.

## Integrity

`integrity.toml` is generated. `dekk agents agent build` synchronizes aggregate
manifests, compiles the TypeScript handler manifest, hashes every recognized
package file in sorted path order, and writes the SHA-256 chain. Do not edit the
seal or generated manifests by hand.

## Verification

Run from the Agents repository root:

```bash
dekk agents doctor
dekk agents frontend setup
dekk agents frontend build
dekk agents frontend typecheck-package examples/agents/gao/tsconfig.json
dekk agents agent sync examples/agents/gao
dekk agents agent lint examples/agents/gao
dekk agents agent build examples/agents/gao
dekk agents test-cli gao_
mkdir -p .apxm/compiled
dekk agents compile-service-canonical examples/agents/gao > .apxm/compiled/gao.air.json
git diff --check
```
