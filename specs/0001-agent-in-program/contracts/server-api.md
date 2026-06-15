# Contract: Server / host I/O seam

The dumb-pipe contract (FR-002, FR-003). The host MUST add no conversational
behavior; it only delivers input and renders output.

## Turn-input endpoint (new)
`POST /v1/conversations/{session_id}/message`
- Body: the user message text (and optional metadata).
- Effect: `park_registry::wake(session_recv_key(session_id), message_value)` —
  delivers the message to the parked recv node of that session's running
  execution. Mirrors the checkpoint-resume route.
- Race: wake-before-register sentinel so a message arriving before the node parks
  is not lost.
- Response: 202/accepted; the reply streams over the session's open output, not
  this response.

## One long-lived execution per session (changed)
- `POST /v1/execute/stream` starts the agent artifact ONCE per session and holds
  the SSE/output stream open across parks (today each turn is a fresh execution).
- Tokens stream from any depth (existing emitter) including from inside the turn
  flow and sub-agents.
- A session→execution registry lets the turn-input endpoint find the running
  execution.

## Admission (changed)
- Spawn grants for in-artifact sub-agents are carried by the program
  (self-contained), or auto-admitted when the artifact declares its own
  SPAWN_AGENT nodes — so a self-contained agent is not rejected for lack of an
  external `--admit` flag.

## Per-session ledger (moved from host)
- Turn caps, per-tool budgets, and the grant set move from host state into a
  runtime ledger keyed by `session_id`, so the host need not track them.

## Minimal host shape (target)
1. Compile/POST the artifact once → open the output stream.
2. For each user line: POST it to the turn-input endpoint.
3. Render streamed tokens.
Nothing else — no transcript, no compaction, no grant/budget bookkeeping, no
loop.
