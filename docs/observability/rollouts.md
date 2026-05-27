# Rollout JSONL transcripts — schema & layout (Phase 14.8.E)

The rollout layer is the durable, per-thread, append-only JSONL store the
apxm runtime writes alongside its in-memory `RunEventBus`. Every event
that lands in the bus is mirrored here so `/v1/runs/...` survives a
restart and regulators can replay a run byte-identically.

This document is the source of truth for the wire schema. The Rust
struct definitions in `apxm-rollout::line` cite this doc; bumps to the
schema must update both in lockstep.

## Layout

```
<APXM_HOME>/sessions/
  rollouts/YYYY/MM/DD/
    rollout-<thread_id>.jsonl              ← main thread
    rollout-<thread_id>/                   ← sidecar
      subagents/agent-<agent_id>.jsonl     ← subagent threads
      blobs/<blake3>.{json,txt}            ← spilled large payloads
  index.sqlite                             ← thread index (derived)
```

The rollout layer sits ALONGSIDE the existing
`.apxm/sessions/skills/<skill-id>/<execution_id>/nodes/<node_id>/`
per-node workspace — heavy artifacts (full prompt/response,
metrics.json) stay there. `RolloutMeta.node_id` cross-references the
two.

## Configuration

- `APXM_ROLLOUT_HOME` — root for `sessions/`. Defaults to `APXM_HOME`,
  then `~/.apxm`.
- `APXM_ROLLOUT_SPILL_THRESHOLD_BYTES` — spill any payload over this
  size to a content-hashed blob. Default `65536`.

## RolloutLine

One line = one `RolloutLine`. The JSON shape is:

```json
{
  "meta": {
    "seq": 0,
    "timestamp": "2026-05-27T10:15:30Z",
    "trace_id": "<thread_id>",
    "span_id": "<uuid>",
    "parent_span_id": null,
    "scope_id": null,
    "source": { "kind": "runtime" },
    "skill": null,
    "uuid": "<uuid>",
    "parent_uuid": null,
    "session_id": "<session_id>",
    "thread_id": "<thread_id>",
    "is_sidechain": false,
    "tool_use_id": null,
    "schema_version": "1.0.0",
    "usage": null,
    "node_id": null
  },
  "payload": { "kind": "<variant>", ... }
}
```

### Meta fields

| Field | Purpose |
|---|---|
| `seq` | Monotonically increasing per file. SSE `Last-Event-ID` uses this. |
| `trace_id` | The thread id. Doubles as the execution id for runtime events. |
| `uuid`, `parent_uuid` | Claude Code parent_uuid pattern — tree edges within the thread. |
| `is_sidechain` | true on every subagent line. |
| `tool_use_id` | Pairs `ToolUse`↔`ToolResult` across threads. |
| `schema_version` | semver. Loaders reject < `MIN_SUPPORTED_VERSION`. |
| `node_id` | When set, resolves to the existing per-node workspace dir. |

### Payload variants

The discriminator is `payload.kind`.

| Kind | Purpose | First-line? |
|---|---|---|
| `session_meta` | Reproducibility envelope. **Always seq=0.** | yes |
| `turn_context` | Sandbox / approval policy snapshot per real user turn. | no |
| `user_message` | Anthropic content-block array for the user turn. | no |
| `assistant_message` | Anthropic content-block array + model + finish_reason. | no |
| `tool_use` | `{ tool_use_id, name, input }`. | no |
| `tool_result` | `{ tool_use_id, content, is_error, latency_ms }`. | no |
| `compacted` | Replacement-history baseline; reverse scan picks the latest. | no |
| `event` | Verbatim `ApxmEvent` envelope (for non-conversational lifecycle). | no |
| `spilled` | `{ blob_ref, bytes_estimate, mime, original_kind }`. | no |

## Reproducibility contract (`session_meta`)

The first line of every rollout file pins:

- `skill_id`, `skill_version`
- `artifact_hash` — blake3 of the `.apxmobj` that ran
- `source_hash` — blake3 of `SKILL.md`
- `air_hash` — blake3 of `skill.air`
- `compiler_version`, `runtime_version`, `apxm_version`
- `args` — verbatim execution args
- `model_provider`, `model_id`, `backend_endpoint`
- `parent_thread_id` + `tool_use_id_in_parent` on subagent threads

These pins are the regulatory replay envelope. `dekk apxm rollout
archive <thread_id>` (Phase 14.8.F) bundles them with the rollout JSONL
and the source files into an air-gapped tarball.

## Spill discipline

When any payload exceeds `APXM_ROLLOUT_SPILL_THRESHOLD_BYTES` (default
64 KiB), the writer:

1. Computes blake3 of the serialized payload.
2. Writes the bytes to `rollout-<thread_id>/blobs/<blake3>.json`
   (atomic via `.tmp` rename so partial blobs never appear after a crash).
3. Emits a `spilled` line in place: `{ blob_ref: <blake3>,
   bytes_estimate, mime, original_kind }`.

Readers fetch the blob on demand via `GET /v1/runs/<thread_id>/blobs/<blob_ref>`.

## Write discipline

- One `flush().await` per line. Codex pattern: fsync trades throughput
  for crash safety; net win for any compliance use case.
- The `SessionMeta` line is written before any other event can land —
  no rollout file can exist without the reproducibility envelope at
  seq=0.

## Read discipline

- `load_rollout(path)` is tolerant: unparseable lines bump
  `LoadStats.parse_errors` and continue.
- `reconstruct_history(items)` reverse-scans to the most recent
  `compacted` marker, then forward-replays the suffix (Codex's
  `rollout_reconstruction.rs` algorithm).
- Forward-compatibility via `#[serde(default)]` on optional fields.

## Index sidecar

`sessions/index.sqlite` mirrors `SessionMeta` rows for fast list/lookup:

```sql
CREATE TABLE threads (
    thread_id        TEXT PRIMARY KEY,
    parent_thread_id TEXT,
    session_id       TEXT,
    started_at       TEXT,
    completed_at     TEXT,
    status           TEXT,
    agent_role       TEXT,
    agent_code       TEXT,
    file_path        TEXT,
    line_count       INTEGER,
    file_bytes       INTEGER
);
```

The index is **derived** — `rebuild_index_from_disk(paths)` fully
regenerates it from the JSONL files. Lose the SQLite, the JSONL files
remain authoritative.

## Versioning

`RolloutMeta.schema_version` is semver. MAJOR bumps require a migration
script in `apxm-rollout/migrations/`. Loaders carry
`MIN_SUPPORTED_VERSION` and reject older rollouts cleanly (no silent
data loss).

## HTTP surface (Phase 14.8.B + 14.8.E)

The observer endpoints under `/v1/runs/...` read the in-memory bus
first, then fall back to the rollout on disk:

- `GET /v1/runs` — lists from `ExecutionStore`, then the SQLite index.
- `GET /v1/runs/<id>/graph` — reads bus, then rollout.
- `GET /v1/runs/<id>/nodes/<node_id>` — reads bus, then rollout.
- `GET /v1/runs/<id>/events` — reads bus, then rollout.
- `GET /v1/runs/<id>/events/stream` — same, supports `Last-Event-ID`
  replay from disk on reconnect.
- `GET /v1/runs/<id>/blobs/<blob_ref>` — returns the spilled blob bytes.
