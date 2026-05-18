# apxm-server

HTTP/SSE gateway exposing the APXM agent runtime over REST, MCP, and A2A protocols.

## Overview

`apxm-server` is an Axum-based HTTP server that wraps `apxm-runtime` and `apxm-compiler` behind a REST v1 API with SSE streaming. It also exposes an MCP 2025-11-25 JSON-RPC endpoint and an A2A v0.3 task interface.

## Binaries

| Binary | Description |
|--------|-------------|
| `apxm-server` | Full HTTP gateway (REST + SSE + MCP + A2A) |
| `apxm-mcp-server` | Standalone MCP server over stdio (JSON-RPC 2.0) |

## REST API (`/v1/`)

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/v1/execute` | POST | Compile and execute a graph |
| `/v1/execute/stream` | POST | Compile and execute with SSE streaming |
| `/v1/generate` | POST | Single LLM generation request |
| `/v1/generate-stream` | POST | Streaming LLM generation |
| `/v1/schema` | GET | JSON schema for structured output |
| `/v1/memory` | GET/POST | Memory read/write |
| `/v1/models` | GET | List available models |
| `/v1/capabilities/register` | POST | Register runtime capabilities |
| `/v1/skills` | GET | List server-owned APXM skill manifests and validation status |
| `/v1/skills/:id` | GET | Get one installed skill by `skill_id` or `skill_id@version` |
| `/v1/skills/:id/validate` | POST | Re-read and validate one installed skill without compiling or executing |
| `/v1/skills/:id/execute` | POST | Execute a validated static skill artifact from the server-owned library |
| `/v1/skills/:id/execute/stream` | POST | Execute a static skill artifact with SSE runtime events |
| `/v1/executions/:execution_id` | GET | Get a server-recorded skill execution status/result |
| `/v1/executions/:execution_id/nodes/:node_id` | GET | Get recorded node output detail for a skill execution |
| `/v1/tasks` | POST | Submit task to queue (CLAIM op) |
| `/v1/tasks/:queue` | GET | List tasks in queue |
| `/v1/tasks/:queue/claim` | POST | Claim a task from queue |
| `/v1/tasks/:id/complete` | POST | Complete a claimed task |
| `/v1/checkpoints` | POST | Create HITL checkpoint (PAUSE) |
| `/v1/checkpoints/:id` | GET | Get checkpoint status |
| `/v1/checkpoints/:id/resume` | POST | Resume/complete a HITL PAUSE checkpoint with human input |

## MCP Endpoint

`POST /v1/mcp` -- JSON-RPC 2.0 per MCP 2025-11-25 specification.

The HTTP MCP endpoint exposes both tools and resources. Skill resources are
served under the `skill://` URI scheme so MCP clients can discover the bundled
skill instructions, manifests, prompts, schemas, and examples without copying a
per-agent skill directory.

Supported resource methods:

- `resources/list` -- list bundled and configured skill resources
- `resources/read` -- read an allowlisted `skill://...` resource

Supported skill resource shapes:

- `skill://<skill-id>/SKILL.md`
- `skill://<skill-id>@<version>/<resource>` when multiple versions of one skill id are installed
- `skill://<skill-id>/_manifest`
- `skill://<skill-id>/prompt.md`
- `skill://<skill-id>/schema.json`
- `skill://<skill-id>/examples/<file>`

Resource resolution is allowlisted to the files above plus `examples/*`;
traversal components and symlinked resources are rejected. If multiple
versions of the same skill id are installed, unversioned reads are rejected as
ambiguous and clients should use `skill://<skill-id>@<version>/...`.

The HTTP MCP endpoint also exposes APXM skill library tools:

- `apxm_skills_list` -- list installed server-owned skill records
- `apxm_skill_get` -- return one skill manifest and validation record
- `apxm_skill_validate` -- re-read and validate one installed skill
- `apxm_skill_call` -- execute a static skill artifact from the server-owned library
- `apxm_plan_as_graph` -- route task-to-graph emission through `model_router`, validate/repair the emitted JSON with compiler feedback, canonicalize common structured-output drift, lower through `AirModule`, compile, optionally execute, and return a compact summary with `trace_id`
- `apxm_trace_fetch` -- fetch execution, episodic, or session trace details by `trace_id`
- `apxm_aam_recall` -- query AAM beliefs/goals/transitions plus runtime memory
- `apxm_evidence_lookup` -- query repo-local `.apxm` claim/evaluation evidence
- `apxm_capability_list` -- list runtime capabilities, LLM backends, and model-router health

Skill inventory prepends the bundled server skill root and then appends roots
configured with repeated `--skill-root <path>` arguments or the
`APXM_SKILL_ROOTS` path list. Requests cannot provide arbitrary roots, artifact
paths, raw AIR, or session roots.

`POST /v1/skills/:id/execute`, `POST /v1/skills/:id/execute/stream`, and
`apxm_skill_call` are intentionally narrow. They only run already compiled
`skill.apxmobj` packages whose manifest validates and whose declared
`artifact_hash` matches the file read at execution time. The server rejects
symlinked artifacts, entry-flow mismatches, unknown request fields such as
`session_root`, process, memory, LLM, agent-spawn calls, and unsupported
side-effect policies. Static `INV_TOOL` nodes are allowed only for manifest
declared capabilities/tools that are registered in the runtime. With omitted or
`read_only` side-effect policy those capabilities must be read-only; with
`sandboxed` policy, side-effectful capabilities must declare sandbox execution
and pass sandbox preflight. Python-backed `INV_TOOL` handlers are rejected.
Generated plans from `apxm_plan_as_graph` follow the same safety shape before
execution: Python tool sections and Python-backed handlers are rejected,
side-effectful direct capabilities are rejected, sandbox-capable tools must
pass sandbox preflight, and HTTP MCP executions are recorded with
`execution_id == trace_id` so `apxm_trace_fetch` can retrieve them directly.
Plan emission also adds runtime capability guidance to the model prompt and
canonicalizes numeric-string and symbolic node ids, named dependency
references, shorthand `depends_on` node references, `attr` aliases, and missing
generated names before typed validation.
Sessions are created under APXM-owned skill session directories using a
generated or simple validated `session_id`, streamed skill runs emit typed
`node_output` and `node_metrics` events, prompt and node-output observability is
redacted to summaries and hashes, and runtime events include the skill id, skill
version, and entry flow.
Completed runs are recorded in the in-memory execution index for
`/v1/executions/:execution_id`. Each update is also snapshotted to
`executions/:execution_id.json` inside the APXM-owned skill session directory.
Typed node outputs and node metrics observed during REST and SSE skill
execution are available at
`/v1/executions/:execution_id/nodes/:node_id` for the lifetime of the server
process.

The raw `/v1/execute` endpoint remains a developer/debug API for direct AIR
experiments. Agent-facing skill clients should use `/v1/skills/:id/execute` or
the MCP `apxm_skill_call` tool so execution goes through the skill manifest,
hash, operation-policy, and server-owned-session checks.

## A2A Endpoints

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/a2a/tasks/send` | POST | Send task to agent |
| `/a2a/tasks/:id` | GET | Get task status |
| `/.well-known/agent.json` | GET | Agent card discovery |

## MCP Stdio Server Tools

The `apxm-mcp-server` binary exposes these tools over stdio:

- `apxm_validate` -- validate AIR text
- `apxm_compile` -- compile AIR text to an optimized artifact
- `apxm_get_contract` -- return the full AIS contract
- `apxm_analyze` -- analyze graph phases, parallelism, and critical path
- `apxm_plan_as_graph` -- emit, validate/repair with compiler feedback, compile, and optionally execute a plan graph via the model router
- `apxm_trace_fetch` -- fetch a compact trace summary by `trace_id`
- `apxm_aam_recall` -- query AAM state and memory
- `apxm_evidence_lookup` -- query `.apxm` evidence stores
- `apxm_capability_list` -- list runtime capabilities and backend health

It also exposes MCP resources:

- `resources/list` -- list `skill://` resources from bundled and configured skill roots
- `resources/read` -- read allowlisted skill resource content

The default bundled skill root is `crates/tools/apxm-server/skills/`, currently
including `skill://apxm-plan-as-graph/SKILL.md`.

The stdio and HTTP MCP surfaces share protocol method names, tool names,
resource fields, schema keys, and tool-result keys through
`src/mcp_protocol.rs` so the wire contract does not drift across transports.

Cross-agent config installation is available through:

```bash
dekk apxm mcp install --auto-detect
```

The installer merges APXM into project `.mcp.json` for Claude Code, into JSON
`mcpServers` blocks for Gemini CLI and Cursor, and into `[mcp_servers.apxm]`
for Codex CLI. Use
`dekk apxm mcp list --auto-detect` to inspect detected configs, or
`dekk apxm mcp uninstall --auto-detect` to remove the registration.

Real-backend plan-as-graph dogfood runs can be repeated with:

```bash
python3 tools/scripts/apxm_plan_as_graph_smoke.py
```

The runner invokes `apxm_plan_as_graph` through the release stdio MCP binary,
uses `dekk apxm vllm service-exec gptoss120b` by default, and writes run
evidence under `.apxm/evaluation/mcp-server/runs/<UTC>/`.

`apxm-mcp-server` can expose skill resources safely, but raw AIR execution is
still a developer/debug path. `apxm_execute` is hidden from `tools/list` and
rejected by `tools/call` by default. Set `APXM_MCP_ENABLE_RAW_EXECUTE=1`
explicitly to expose and enable it for local debugging. Safe static skill
execution should continue to use the HTTP MCP `apxm_skill_call` tool or REST
skill execution routes so execution goes through manifest validation, hash
checks, and sandbox preflight. This stdio-only raw-execution gate is separate
from the raw HTTP `/v1/execute` developer/debug API described above.

## Dependencies

| Crate | Purpose |
|-------|---------|
| apxm-compiler | Graph parsing and compilation |
| apxm-runtime | DAG execution engine |
| apxm-backends | LLM provider registry |
| apxm-artifact | Artifact serialization |
| apxm-skill | Shared skill manifests, validation reports, hashes, and execution provenance |
| apxm-core | Shared types, error codes, constants |
| apxm-credentials | Backend credential lookup |
