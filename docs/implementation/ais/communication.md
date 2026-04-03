# Communication Operations

These operations handle inter-agent messaging, task distribution, human-in-the-loop suspension, and agent lifecycle. They span multiple categories: **Communication** (COMMUNICATE, CLAIM, PAUSE), **Coordination** (SPAWN_AGENT, DELEGATE, NEGOTIATE, REGISTER_CAPABILITY, AUTONOMOUS).

## COMMUNICATE

Sends a message to another agent. The input token from upstream edges provides the message content.

| Field | Required | Description |
|-------|----------|-------------|
| `recipient` | yes | Target agent name (or URL for http/https protocol) |
| `protocol` | no | Dispatch mode: `local` (default), `http`, `https`, `acp`, `broadcast` |

**Latency tier:** Medium.

**Protocol dispatch:**
- `local` -- in-process sub-flow invocation (default).
- `http` / `https` -- external APXM agent over HTTP.
- `acp` -- ACP subprocess via ProcessTable. The recipient must have been spawned with SPAWN_AGENT + `profile`.
- `broadcast` -- fan-out to all registered agents.

```json
{"id": 3, "op": "COMMUNICATE", "attributes": {"recipient": "reviewer", "protocol": "acp"}}
```

Multiple COMMUNICATE nodes targeting the same ACP agent reuse the same session, maintaining conversation context across turns.

## CLAIM

Atomically claims a task from a distributed work queue managed by the APXM server.

| Field | Required | Description |
|-------|----------|-------------|
| `queue` | yes | Queue name to claim from |
| `lease_ms` | no | Lease duration in ms (default: 60000) |
| `max_wait_ms` | no | Max time to wait for a task (default: 5000) |
| `server_url` | no | Override `APXM_SERVER_URL` env var |

**Latency tier:** Low. **Category:** Communication.

```json
{"id": 2, "op": "CLAIM", "attributes": {"queue": "review_tasks", "lease_ms": 30000}}
```

## PAUSE

Creates a checkpoint and suspends execution until a human resumes it via the APXM server API.

| Field | Required | Description |
|-------|----------|-------------|
| `message` | yes | Human-readable message explaining the pause |
| `checkpoint_id` | no | Stable checkpoint ID (auto-generated if omitted) |
| `timeout_ms` | no | Max wait in ms (0 = indefinite, default: 0) |
| `poll_interval_ms` | no | Polling interval in ms (default: 2000) |
| `notification_url` | no | Webhook URL to notify on pause creation |
| `server_url` | no | Override `APXM_SERVER_URL` env var |

**Latency tier:** High (human-dependent). **Category:** Communication.

```json
{"id": 5, "op": "PAUSE", "attributes": {"message": "Please review the analysis before proceeding"}}
```

## SPAWN_AGENT

Creates a new agent instance at runtime. Without `profile`, registers a local process for flow-based agents. With `profile`, spawns a real ACP subprocess (Claude, Codex, Gemini, etc.) via the AgentSpawner.

| Field | Required | Description |
|-------|----------|-------------|
| `agent_name` | yes | Name for the new agent |
| `profile` | no | ACP agent profile (e.g. `claude`, `codex`). Spawns ACP subprocess when present |
| `mode` | no | Agent mode (e.g. `architect`, `code`) |
| `model` | no | Model override (e.g. `claude-sonnet-4`) |
| `cwd` | no | Working directory for the subprocess |
| `capabilities` | no | List of capabilities for the new agent |
| `goals` | no | Initial goals for the new agent |

**Latency tier:** Variable. **Category:** Coordination.

```json
{"id": 1, "op": "SPAWN_AGENT", "attributes": {"agent_name": "reviewer", "profile": "claude", "mode": "architect"}}
```

## DELEGATE

Delegates a task to a sub-agent for execution. Returns a task handle.

| Field | Required | Description |
|-------|----------|-------------|
| `task_spec` | yes | Description of the task to delegate |
| `target_agent` | yes | Name of the agent to delegate to |

**Latency tier:** Variable. **Category:** Coordination.

## NEGOTIATE

Multi-party negotiation protocol among a set of agents. Circulates a proposal for configurable rounds and returns the consensus result.

| Field | Required | Description |
|-------|----------|-------------|
| `parties` | yes | List of agent names participating |
| `proposal` | yes | The proposal to negotiate on |
| `max_rounds` | no | Maximum negotiation rounds (default: 3) |

**Latency tier:** Variable. **Category:** Coordination.

## REGISTER_CAPABILITY

Dynamically registers a new capability in the runtime's CapabilityRegistry, making it available for INV operations.

| Field | Required | Description |
|-------|----------|-------------|
| `capability_name` | yes | Name for the capability |
| `description` | no | Human-readable description |
| `parameters_schema` | no | JSON schema for capability parameters |

**Latency tier:** Low. **Category:** Coordination.

## AUTONOMOUS

Switches a sub-graph region to model-driven (unstructured) execution. Currently a stub that passes through input unchanged.

| Field | Required | Description |
|-------|----------|-------------|
| `region` | no | Name of the autonomous execution region |

**Latency tier:** Variable. **Category:** Coordination.
