# Data Model — 0002-apxm-chat-thin-clients

Entities from `spec.md` Key Entities. Validation rules are server-enforced.

## Session

| Field | Description |
|---|---|
| `session_id` | Stable conversation/subject lane across turns/runs |
| `turn_cap` | Optional max turns; enforced at recv re-arm |
| `tool_budgets` | Map tool/capability id → remaining invocations |
| `grants` | Set of capability ids approved for session scope |
| `approval_grants` | Cached permission decisions (tool + args fingerprint) |
| `linked_executions` | Active/historical `execution_id` handles |
| `history_index` | Pointer into server-owned transcript (not client copy) |

**State transitions:** `active` → `compacting` → `active`; `active` → `cancelled`;
turn cap exceeded → `turn_denied` (typed error, session remains for observation).

**Relationships:** 1 session → N runs (`execution_id`); memory scope uses `session_id`
when present (review §7, `context.rs`).

## Run (`execution_id`)

| Field | Description |
|---|---|
| `execution_id` | Cancellable handle for one graph execution |
| `session_id` | Optional parent session |
| `graph_id` | Stable graph identity in runtime |
| `status` | `running` \| `completed` \| `failed` \| `cancelled` |
| `cancel_handle` | Registered on every submit path (FR-009) |

## Permission Event

| Field | Description |
|---|---|
| `permission_id` | Correlates prompt and response |
| `execution_id` | Run blocked pending decision |
| `capability_id` | Tool/capability requiring approval |
| `args_summary` | Human/model-visible argument digest |
| `timeout_at` | Server-side wait bound |
| `session_grant_eligible` | Whether "approve for session" is offered |

**Flow:** emit on stream → block tool → await `PermissionResponse` → allow/deny →
optional session grant cache in ledger.

## Typed Error

| Field | Description |
|---|---|
| `class` | `program_fault` \| `server_fault` |
| `code` | Stable machine-readable identifier |
| `message` | Human-readable detail |
| `recovery_hint` | Optional model-facing guidance |
| `http_status` | 4xx program / 5xx server |

## Generated Client Artifact

| Field | Description |
|---|---|
| `openapi_version` | Exported schema revision |
| `fixture_hash` | CI diff-test baseline |
| `crate_version` | `apxm-client` semver aligned with server |

No client-side mirror of Session or ledger entities — consumers call session API only.
