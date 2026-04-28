# apxm-server

HTTP/SSE gateway exposing the APXM agent runtime over REST, MCP, and A2A protocols.

## Overview

`apxm-server` is an Axum-based HTTP server that wraps `apxm-runtime` and `apxm-compiler` behind a REST v1 API with SSE streaming. It also exposes an MCP 2025-11-05 JSON-RPC endpoint and an A2A v0.3 task interface.

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

`POST /v1/mcp` -- JSON-RPC 2.0 per MCP 2025-11-05 specification.

The HTTP MCP endpoint also exposes APXM skill library tools:

- `apxm_skills_list` -- list installed server-owned skill records
- `apxm_skill_get` -- return one skill manifest and validation record
- `apxm_skill_validate` -- re-read and validate one installed skill
- `apxm_skill_call` -- execute a static skill artifact from the server-owned library

Skill inventory is configured with repeated `--skill-root <path>` arguments or
the `APXM_SKILL_ROOTS` path list. Requests cannot provide arbitrary roots,
artifact paths, raw AIR, or session roots.

`POST /v1/skills/:id/execute`, `POST /v1/skills/:id/execute/stream`, and
`apxm_skill_call` are intentionally narrow. They only run already compiled
`skill.apxmobj` packages whose manifest validates and whose declared
`artifact_hash` matches the file read at execution time. The server rejects
symlinked artifacts, entry-flow mismatches, unknown request fields such as
`session_root`, process, memory, LLM, agent-spawn calls, and unsupported
side-effect policies. Static `INV_TOOL` nodes are allowed only for manifest
declared capabilities/tools that are registered in the runtime and marked
read-only; Python-backed `INV_TOOL` handlers are rejected. Sessions are created
under APXM-owned skill session directories using a generated or simple validated
`session_id`, streamed skill runs emit typed `node_output` and `node_metrics`
events, and completed
runs are recorded in the in-memory execution store for
`/v1/executions/:execution_id`. Typed node outputs and node metrics observed
during REST and SSE skill execution are available at
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
- `apxm_execute` -- compile and execute AIR text in one shot
- `apxm_get_contract` -- return the full AIS contract

## Dependencies

| Crate | Purpose |
|-------|---------|
| apxm-compiler | Graph parsing and compilation |
| apxm-runtime | DAG execution engine |
| apxm-backends | LLM provider registry |
| apxm-artifact | Artifact serialization |
| apxm-core | Shared types, error codes, constants |
| apxm-credentials | Backend credential lookup |
