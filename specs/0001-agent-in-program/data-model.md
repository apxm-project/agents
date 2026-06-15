# Data Model: Agent in One Program

Entities, fields, relationships, and state transitions implied by the
requirements. Stored representations are noted only where they already exist.

## Agent program (artifact)
One authored definition compiled to **one multi-flow artifact** (a module with
several flows).
- **entry flow** (`main`) — the conversation loop.
- **turn flow** — the author-defined per-turn sub-DAG (recall → inject → ask →
  hooks → delegate → remember → compaction → done).
- **sub-agent flows** — zero or more `<Agent>.main` / `<Agent>.delegate`.
- **hooks manifest** — sidecar descriptors (handler_id, module, qualname, event,
  match, mode), embedded in the artifact like the tools sidecar.
- Relationship: entry flow re-arms the turn flow once per user message; turn flow
  may delegate to a sub-agent flow.
- Invariant (FR-001/FR-002/FR-014): a referenced hook/sub-agent/capability that
  is not present in the artifact is a compile/admission error, never a silent
  drop.

## Turn
One user message and its resulting reply.
- Fields: incoming user message; recalled context window; system prompt (possibly
  dataflow-injected); tool calls + results; optional delegation result; reply.
- Transition: `parked(awaiting message) → running(turn flow) → reply streamed →
  re-arm(parked)` until session end. No re-run of prior turns (FR-012, SC-007).

## Lifecycle rule (hook)
An author behavior bound to a moment.
- Fields: `event ∈ {session_start, pre_turn, post_turn, pre_ask, post_ask,
  pre_tool, post_tool}`; `match` (glob over tool/op name; default `*`);
  `mode ∈ {observe, gate}`; `handler_id` (Python handler, same registry as tools).
- Rules: `gate` valid only on `pre_*` events (only those can Allow/Deny/EditArgs);
  `observe` on any.
- Failure semantics (resolved default): `gate` failure → fail-closed (deny +
  surface); `observe` failure → surface-and-continue.
- Decision shape for `pre_tool`: `Allow | Deny(reason) | EditArgs(args)`; for
  `pre_ask`: may prepend/replace system prompt; for `post_*`: may transform the
  result/reply.

## Sub-agent
A specialist defined in the same program.
- Fields: name, instructions, optional model/route, optional tools.
- Resolution: by name to a sibling flow in the artifact (`<name>.main`/
  `.delegate`) or, absent a flow, an inline LLM dispatch from `agent_info`
  written at spawn (the DELEGATE inline fallback). Default execution in-process;
  ACP-profile sub-agents opt-in.

## Session memory
Cross-turn state scoped to one conversation.
- Fields: history/recent-window, running summary, deliberately stored facts;
  per-session budget + grant ledger (moves from host to runtime).
- Key: stable session identifier (`memory_scope()` = session id). Absent → memory
  degrades predictably (surfaced, not crashed) per Assumptions.
- Transition: `pre_ask` recalls a recent window; `post_turn` appends; compaction
  folds older turns into the summary when over the working limit (FR-007/FR-008),
  keeping author-marked facts.

## Park wait-key
The rendezvous that makes a turn wait for the next user message without pinning a
worker.
- Fields: `session_recv_key` (per session), pending value (user message).
- Transition: turn flow's recv node returns `Parked(wait_key)` → worker yields
  lane + permit + releases admission slot → host POSTs message → `wake(wait_key,
  message)` → recv node completes with the message as its output → turn runs →
  loop re-arms a fresh recv. Wake-before-register sentinel handles the race.

## Skill / capability
Installed functionality discoverable by description.
- Reused as-is; the agent ranks the visible set by description and invokes by id.
  No new storage; this feature only adds a first-class discovery helper and wires
  the real capability into the canonical turn.
