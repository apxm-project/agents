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
| `/v1/tasks` | POST | Submit task to queue (CLAIM op) |
| `/v1/tasks/:queue` | GET | List tasks in queue |
| `/v1/tasks/:queue/claim` | POST | Claim a task from queue |
| `/v1/tasks/:id/complete` | POST | Complete a claimed task |
| `/v1/checkpoints` | POST | Create HITL checkpoint (PAUSE) |
| `/v1/checkpoints/:id` | GET | Get checkpoint status |
| `/v1/checkpoints/:id/resume` | POST | Resume from checkpoint |

## MCP Endpoint

`POST /v1/mcp` -- JSON-RPC 2.0 per MCP 2025-11-05 specification.

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
