# Telegram Coding Bot — Execution Flow and Implementation Reference

This document synthesizes a full investigation of the stack for running a
Telegram coding bot in APXM without Studio. It covers topology, execution
flow, compilation pipeline, credential model, root-cause analysis of observed
errors, the correct implementation pattern, and open gaps.

---

## 1. Full Stack Topology

```
 ┌────────────────────────────────────────────────────────────┐
 │  Telegram servers                                          │
 │  POST https://<host>:9090/webhooks/telegram  (HMAC-signed) │
 └────────────────────────────┬───────────────────────────────┘
                              │ inbound webhook
                              ▼
 ┌────────────────────────────────────────────────────────────┐
 │  apxm-os  :9090                                            │
 │  os-listeners/webhook.rs   — HMAC verify, CueEvent build   │
 │  os-supervisor             — durable inbox, dedup, retry   │
 │  os-dispatch               — act-mode → skill dispatch     │
 │  os-client                 — POST /v1/skills/{id}/execute  │
 └────────────────────────────┬───────────────────────────────┘
                              │ HTTP POST /v1/skills/{id}/execute
                              │ (SkillExecuteRequest, args=[envelope_json])
                              ▼
 ┌────────────────────────────────────────────────────────────┐
 │  apxm-server  :18800                                       │
 │  /v1/skills/{id}/execute   — find .apxmobj, verify hash    │
 │  /v1/compile-artifact      — registry-aware compile        │
 │  /v1/capabilities          — registered capability list    │
 │  CapabilitySystem          — builtins + pack tools         │
 │  AgentSpawner              — ACP subprocess launch         │
 └──────────┬────────────────────────────────────────────────┘
            │ GET /v1/connections/{id}/token  (credential resolve)
            │ POST /v1/connections/{id}/proxy  (outbound call)
            ▼
 ┌────────────────────────────────────────────────────────────┐
 │  apxm-auth  :18810                                         │
 │  connection store          — bot token at rest             │
 │  proxy endpoint            — injects token, forwards       │
 └────────────────────────────┬───────────────────────────────┘
                              │ HTTPS to Telegram Bot API
                              ▼
                    https://api.telegram.org/bot<token>/sendMessage

 apxm-studio  :18802  (not involved in the OS-dispatch flow)
```

Key independence boundary: apxm-os has zero Cargo dependencies on
apxm-server or apxm-auth. The two seams are HTTP only: os-client
→ apxm-server (skill execute), and apxm-server → apxm-auth (credential
resolve/proxy).

---

## 2. Execution Flow

Numbered steps from Telegram sending the webhook to Telegram receiving
the reply.

### 2a. Respond-mode flow (canonical, no outbound token in skill)

This is the pattern confirmed live in the `telegram-reply-20260613T191340`
session. The cue sets `respond = true`.

```
1.  Telegram POST /webhooks/telegram
      Headers: X-Telegram-Bot-Api-Secret-Token: <hmac-secret>
      Body:    {"message": {"chat": {"id": 12345}, "text": "/code ..."}}

2.  os-listeners/webhook.rs
      - HMAC-verify against cue's secret_token
      - Extract subject via subject_pointer (e.g. "/message/chat/id" → "12345")
      - Build CueEvent {cue_mode="act", cue_skill="telegram-coding-bot",
          subject="12345", payload=<full Telegram JSON>}
      - Send event to agent's mpsc::Sender<CueEvent>

3.  os-supervisor agent_loop (lib.rs:1828)
      - Receive from rx
      - transport.publish_inbox() — durable write to disk
      - process_event()

4.  os-supervisor process_event (lib.rs:1871)
      - stamp CloudEvents envelope
      - transport.claim_seen() — dedup check
      - Acquire session-order gate + semaphore permit
      - Spawn handler task → dispatch_with_retry()

5.  os-supervisor dispatch_with_retry (lib.rs:2042)
      - Up to 4 attempts (1 initial + 3 retries)
      - Backoff: min(250ms × 2^(attempt-1), 8000ms) + jitter
      - Calls Dispatcher::handle_event_with_detach(event, detach, key)

6.  os-dispatch handle_event_with_detach (lib.rs:125)
      - Detects cue_mode == "act", cue_skill == "telegram-coding-bot"
      - Delegates to dispatch_act("telegram-coding-bot", event, ...)

7.  os-dispatch dispatch_act (lib.rs:183)
      - Check allowed_skills gate
      - Build envelope JSON:
          {"event": <CueEvent>, "agent_context": {"agent_id": ..., "beliefs": ...}}
      - Build SkillExecuteRequest:
          {args: [envelope_json],
           session_id: "apxm-os-<agent-name>-12345",
           sandbox_hint, detach, idempotency_key, correlation_id}
      - Call client.execute("telegram-coding-bot", &req)

8.  os-client execute (lib.rs:102)
      - POST http://127.0.0.1:18800/v1/skills/telegram-coding-bot/execute
      - connect_timeout = 2s, request_timeout = 30s
      - Body: SkillExecuteRequest as JSON

9.  apxm-server /v1/skills/{id}/execute handler (skills.rs:536)
      - If detach=true: prepare_skill_execution_with_reservation()
          → spawn_detached_skill_execution(), return 202
      - If detach=false: execute_skill_by_id()

10. apxm-server prepare_skill_execution (skills.rs:1083)
      - skill_library.find_executable("telegram-coding-bot")
          → scans skill_roots for skill.toml with skill_id match
          → loads skill.apxmobj from <package_dir>/skill.apxmobj
          → verifies BLAKE3 hash against skill.toml artifact_hash
          → deserializes to Artifact, caches by (skill_id, artifact_hash)
      - validate_static_skill_admission(): checks allowed_tools, side_effect_policy,
          every INV_TOOL node in the artifact

11. apxm-runtime execute_artifact_with_session_emitter_and_metadata
      - Executes the compiled AIS DAG node by node
      - For an ask() node: calls the configured LLM backend
      - For an inv_tool node: dispatches to the named capability handler
      - Session dir: <apxm_paths>/sessions/skills/telegram-coding-bot/
                     apxm-os-<agent-name>-12345/
        (per-conversation context persists across deliveries from same chat_id)
      - Skill returns {method: "sendMessage", chat_id: 12345, text: "..."}

12. os-listeners/webhook.rs (respond=true path)
      - Returns the skill's output JSON as the HTTP 200 response body
        to Telegram's incoming POST
      - Telegram executes sendMessage server-side using its own bot token
      - No outbound token required in the skill at all

13. Telegram delivers the reply message to the user
```

### 2b. Proactive outbound flow (for replies not in-band with the webhook)

Used when the skill needs to send a message outside the respond window
(e.g., after a long-running coding agent session completes). Requires
`detach = true` on the OS dispatch and `inv_tool "telegram.send_message"`
in the compiled AIR.

```
Steps 1–10: identical to 2a above, with detach=true in SkillExecuteRequest.

11. apxm-server returns 202 immediately after reserving the execution slot.

12. Detached skill execution runs asynchronously.
      - AIS DAG executes; may include ais.spawn_agent {profile="codex"} node
        followed by ais.communicate to receive the coding result
      - After coding completes: inv_tool node with capability="telegram.send_message"
        - params_json: {"credential": "telegram/main", "chat_id": "...", "text": "..."}

13. APXM_RESOLVE_CREDENTIALS path (if enabled on server):
      - inject_resolved_credentials (execute.rs) calls apxm-auth:
          GET /v1/connections/telegram%2Fmain/token
      - Drops "credential" key from params_json
      - Injects Authorization: Bearer <bot_token> into headers

14. ProviderCallCapability::execute
      - POSTs to apxm-auth /v1/connections/telegram%2Fmain/proxy
      - apxm-auth substitutes token into URL template:
          https://api.telegram.org/bot<token>/sendMessage
      - Telegram delivers the reply

15. Skill returns; session dir persists conversation context.
```

---

## 3. Compilation Pipeline

### 3a. Two compile paths and when to use each

**Path A — standalone `dekk apxm compile` (local, no server)**

Calls `generate_artifact_with_manifest(None, manifest)`. The `known_caps`
set passed to `tool_binding_check_dag` is empty (hardwired, no flag overrides
this). Any `inv_tool` node whose capability is not a builtin fails E712. Use
this path only for skills that use nothing but builtins (`ask`, `inv_tool
"provider.call"` if registered as builtin, `spawn_agent`, `communicate`,
`return`, `switch`, `loop`).

**Path B — `POST /v1/compile-artifact` (server-side, registry-aware)**

Calls `registered_capability_names(state)` first, which queries
`state.runtime.capability_system().list_capabilities()`. This set includes:
- All builtin Rust capabilities (registered in `runtime_setup.rs`)
- All pack/connector capabilities loaded from installed `tools.toml` files
  at server startup via `register_pack_tools()`
- Any capability registered via the POST /v1/capabilities API

The wire schema is `{"air": "<AIR text>"}`. The response is raw `.apxmobj`
bytes (`application/octet-stream`). This is the path Studio uses for
compile-on-deploy. Use this path for any skill that uses pack capabilities
(`telegram.send_message`, etc.).

### 3b. Correct pipeline for a Python skill using telegram.send_message

```
1. Ensure the telegram pack is installed:
     ~/.apxm/libs/telegram/tools.toml  (must contain [[tool]] telegram.send_message)
   Verify the server sees it:
     curl http://localhost:18800/v1/capabilities | grep telegram

2. Emit AIR from the Python script:
     APXM_EMIT_AIR=1 python my_skill.py   (prints AIR to stdout)
   Or capture it to a file.

3. POST the AIR to the server:
     curl -X POST http://localhost:18800/v1/compile-artifact \
       -H "Content-Type: application/json" \
       -d '{"air": "<air_text>"}' \
       --output skill.apxmobj

4. Place the artifact:
     mkdir -p ~/.apxm/skills/telegram-coding-bot/
     cp skill.apxmobj ~/.apxm/skills/telegram-coding-bot/

5. Write skill.toml (minimum required fields):
     skill_id = "telegram-coding-bot"
     version  = "0.1.0"
     entry_flow = "main"
   Optionally compute and record artifact_hash:
     b3sum skill.apxmobj   → record as artifact_hash = "blake3:<hex>"

6. Verify:
     dekk apxm doctor
```

### 3c. The E712 and E900 errors

**E712 (tool binding check)**: fires during compilation when an `inv_tool`
node names a capability not in the known_caps set. With standalone compile
(`dekk apxm compile`), known_caps is empty, so any pack capability fails E712.
Fix: compile via `POST /v1/compile-artifact` instead.

**E900 (null return from FFI)**: fires from `ffi/utils.rs:handle_null_result`
when the C++ MLIR compiler returns a null pointer AND the C++ error queue is
empty. This means an MLIR pass failed without emitting a diagnostic. It is
NOT caused by `spawn_agent` or `communicate` being unrecognized (both are
fully wired: wire indices 35 and 9, cases in `ArtifactEmitter.cpp`,
`is_allowed_static_skill_op` permits both, runtime dispatcher handles both).
The actual cause in the reported case is most likely the MLIR op verifier
rejecting the mixed literal+SSA form in the `communicate` op (string literal
recipient attr with an SSA `%coder` operand), or the `NormalizeAgentGraph`
pass rejecting the inter-op value constraints. Debug by inspecting the server
log for MLIR stderr output captured before the null return.

---

## 4. The Credential Model

### 4a. Where the bot token lives

The bot token must be stored in apxm-auth as a connection, not in the skill,
the AIR, or any environment variable visible to the skill. apxm-auth stores it
at rest under a connection id (e.g., `telegram/main`). The skill author
references the connection id as a string; the actual token never enters the
AIR text or the Python source.

### 4b. How the token flows in the proactive outbound pattern

```
AIR: inv_tool node
  capability = "telegram.send_message"
  params_json = '{"credential": "telegram/main", "chat_id": "...", "text": "..."}'

→ apxm-server inject_resolved_credentials (execute.rs):
     GET /v1/connections/telegram%2Fmain/token  (to apxm-auth :18810)
     drops "credential" key from params_json
     injects Authorization: Bearer <bot_token> into headers

→ ProviderCallCapability::execute:
     POST /v1/connections/telegram%2Fmain/proxy  (to apxm-auth)
     apxm-auth injects token into URL template, forwards to Telegram
```

The token never appears in: the Python skill source, the emitted AIR, the
compiled artifact, the `SkillExecuteRequest` that OS sends, or any log line
from the skill runtime.

### 4c. Why http_post with a hardcoded bot token is wrong

`http_post` with a hardcoded token puts the credential directly into the AIR
as a string literal. This means it appears in:
- The compiled `.apxmobj` artifact on disk (readable by anyone with file access)
- The AIR source checked into version control
- Any execution log that echoes params_json

It also bypasses the apxm-auth rotation, audit, and proxy machinery. The
correct pattern is the connection-id reference resolved at dispatch time by the
server.

### 4d. The respond-mode alternative (no token in skill at all)

When `respond = true` is set on the cue, the skill returns a JSON object
`{method: "sendMessage", chat_id: <id>, text: <text>}` and the OS webhook
listener returns that as the HTTP 200 body to Telegram's incoming POST.
Telegram executes the Bot API call from its own servers using its own knowledge
of the bot token (verified via the `X-Telegram-Bot-Api-Secret-Token` HMAC
header on the incoming webhook). No token is needed in the skill, the AIR, or
the server at all for this path.

---

## 5. Root Cause of "apxm-server unreachable"

The error originates at `os-client/src/lib.rs` in `client.execute()`, which
uses `connect_timeout = 2s` (`APXM_SERVER_CONNECT_TIMEOUT`, line 23). A TCP
`Connection refused` response (wrong port) returns well within that window.

**The precise cause:** each agent manifest carries its own `target_server`
field (default `http://127.0.0.1:18800`, from `os-core/src/agent.rs` line
116). The OS supervisor reads this field at agent registration time
(`lib.rs:1319`) and builds the `ApxmServerClient` from it. If the manifest
was written with a hardcoded `18800` but the server was started on an
ephemeral port (the worktree-parallel orchestration assigns ports via
`APXM_RUNTIME_DIR/<svc>.json` and `bind :0`), the OS client attempts to
connect to a port with no listener and gets `Connection refused` immediately.

Two OS instances were confirmed running (`pids 2904095 and 3096119`). If the
agent manifest used by one instance points to a port that belongs to the other
instance's server (or to a server that is not running at all), the connect
fails and all four retry attempts exhaust within seconds, dead-lettering the
event.

This is NOT a schema mismatch (which would produce a 400 from the server, not
a connect failure), NOT a timeout from a slow server (which would be 30s via
`APXM_SERVER_REQUEST_TIMEOUT`), and NOT related to the skill or credential
configuration.

**Fix:** set `target_server` in the agent manifest to the actual port the
target apxm-server instance is listening on, read from
`$APXM_RUNTIME_DIR/apxm-server.json` after startup.

---

## 6. The Correct Implementation

### 6a. What ops are supported in a compiled skill artifact

All of the following are fully supported in a compiled `.apxmobj` artifact
(wired in definitions.rs, emittable by ArtifactEmitter.cpp, permitted by
`is_allowed_static_skill_op`, dispatched by the runtime):

- `ais.ask` — LLM call
- `ais.inv_tool` — capability dispatch (including pack capabilities)
- `ais.spawn_agent` — ACP subprocess launch (profile="codex" or "claude")
- `ais.communicate` — send/receive to a spawned agent
- `ais.switch` — conditional branch
- `ais.loop` — iteration
- `ais.return` — skill output
- `ais.register_capability` — register a Python @tool (Python skills)

What is NOT supported in an act-mode compiled skill dispatched synchronously:
`spawn_agent {profile="codex"}` inside a synchronous (non-detached) execution
will block for the full codex session duration, which exceeds the
`APXM_SERVER_REQUEST_TIMEOUT` of 30s. Use `detach = true` for any skill that
spawns a coding agent.

### 6b. The respond-mode skill (correct for short replies)

```python
# telegram_reply_skill.py
# Respond-mode: returns {method, chat_id, text} as JSON.
# OS webhook listener returns this as HTTP 200 body to Telegram.
# No bot token needed anywhere in the skill.

import json
from apxm import compile, ask

@compile()
def main(payload: str) -> str:
    event = json.loads(payload)
    # payload is args[0] from the OS envelope:
    # {"event": <CueEvent>, "agent_context": {...}}
    cue_event = event["event"]
    telegram_msg = cue_event["payload"]  # the raw Telegram webhook body
    chat_id = telegram_msg["message"]["chat"]["id"]
    user_text = telegram_msg["message"].get("text", "")

    reply = ask(f"The user asked: {user_text}. Reply helpfully.")

    return json.dumps({
        "method": "sendMessage",
        "chat_id": chat_id,
        "text": reply
    })
```

Compile with: `dekk apxm compile telegram_reply_skill.py` (no pack caps, so
standalone compile works). Place result at
`~/.apxm/skills/telegram-coding-bot/skill.apxmobj` with matching `skill.toml`.

### 6c. The detached coding agent skill (correct for long-running Codex)

This pattern is for the full Telegram coding bot: receive the user's code
request, spawn Codex via ACP, wait for the result, send the reply via
`telegram.send_message`.

The skill must be compiled via `POST /v1/compile-artifact` (because it uses
`telegram.send_message` from the pack). The cue must set `detach = true`.

AIR outline (the Python `@compile` decorator emits this; shown for clarity):

```
// entry flow: main
// args[0] = OS envelope JSON (event + agent_context)
%payload = ais.ask "Extract the code request from {args[0]}"
             {model = "codex", task = %payload}    // or use spawn_agent
%coder   = ais.spawn_agent {profile = "codex", task = %payload}
%result  = ais.communicate %coder
%_       = ais.inv_tool "telegram.send_message"
             {params_json = '{"credential": "telegram/main",
                              "chat_id": "<extracted>",
                              "text": "<result>"}'}
```

The `spawn_agent` + `communicate` + `inv_tool` chain is the correct pattern.
The E900 encountered during investigation is NOT a fundamental limitation; it
is a verifier rejection of the specific AIR shape tried. The fix is to ensure
the `communicate` op's recipient is expressed as an SSA operand only (no mixed
literal+SSA form).

### 6d. Agent manifest

```toml
# ~/.apxm/agents/telegram-coding-bot.toml
[agent]
name            = "telegram-coding-bot"
target_server   = "http://127.0.0.1:18800"   # must match actual server port
allowed_skills  = ["telegram-coding-bot"]

[[cues]]
name            = "on-telegram-message"
on_event        = "skill://telegram-coding-bot"
cue_mode        = "act"
respond         = false        # true for respond-mode; false for detached outbound
detach          = true         # required for Codex (long-running)
subject_pointer = "/message/chat/id"
dedup_source    = "/message/message_id"
connection      = "telegram/main"

[[listeners]]
kind            = "webhook"
path            = "/webhooks/telegram"
secret_token    = "${TELEGRAM_WEBHOOK_SECRET}"
```

### 6e. Skill.toml

```toml
# ~/.apxm/skills/telegram-coding-bot/skill.toml
skill_id   = "telegram-coding-bot"
version    = "0.1.0"
entry_flow = "main"
# artifact_hash = "blake3:<hex>"  # fill after compilation
```

---

## 7. Open Issues and Gaps

### 7a. E900 on spawn_agent + communicate AIR shape

**Status:** not yet resolved. The C++ `ArtifactEmitter.cpp` has cases for both
ops and the runtime dispatcher handles them. The E900 is a pre-emission MLIR
pass failure (most likely `CommunicateOp` verifier rejecting the specific AIR
form tried, or `NormalizeAgentGraph` rejecting the inter-op value constraints).

**What is unknown:** the exact AIR shape that the verifier accepts for a
`communicate` op that receives from a `spawn_agent` result token. The MLIR op
definition for `CommunicateOp` and its verifier constraints have not been read;
the correct SSA form is not confirmed.

**Next step:** read `apxm-ais/src/mlir/lib/Dialect/AIS/AIS.cpp` verifier for
`CommunicateOp`; emit a minimal reproduce AIR; POST to `/v1/compile-artifact`
and capture full server stderr.

### 7b. AgentSpawner configuration

**Status:** if apxm-server is not started with ACP spawn capability enabled,
`spawn_agent.rs` line 142 returns "No AgentSpawner configured. Cannot spawn
ACP agent." This is a runtime failure, not a compile failure. Whether the
current stack startup (`dekk apxm-studio stack up` or equivalent) enables the
AgentSpawner is not confirmed.

**Next step:** check apxm-server startup args and confirm AgentSpawner is
wired in `runtime_setup.rs` for the current stack.

### 7c. Port mismatch under worktree-parallel orchestration

**Status:** two OS instances are running (`pids 2904095 and 3096119`). The
`target_server` in each agent manifest must point to the specific
apxm-server instance that has the skill registered. If manifests have
hardcoded `18800` and the server is on a different ephemeral port, all
skill dispatches fail immediately (2s connect timeout, then dead-letter
after 4 attempts).

**Fix:** read `$APXM_RUNTIME_DIR/apxm-server.json` after server startup to
get the actual bound port; write it into the agent manifest's `target_server`
before registering the agent with apxm-os.

### 7d. 30s timeout for synchronous Codex sessions

**Status:** the `APXM_SERVER_REQUEST_TIMEOUT` is 30s (`os-client/src/lib.rs`
line 27). A Codex ACP session that takes longer (typical for non-trivial
coding tasks) will cause the OS client to get a timeout error, trigger the
retry loop (4 attempts, up to ~24s additional backoff), and then dead-letter
the event while the Codex session continues running detached on the server.

**Fix:** always set `detach = true` on the cue for skills that spawn Codex.
The server acks 202 immediately and the skill sends the Telegram reply
asynchronously via `inv_tool "telegram.send_message"` after Codex completes.

### 7e. Session context across turns

**Status:** the OS mints `session_id = "apxm-os-<agent-name>-<chat_id>"` using
the Telegram `chat_id` extracted via `subject_pointer`. This correctly threads
per-conversation context across multiple webhook deliveries from the same chat.
The session dir at `<apxm_paths>/sessions/skills/telegram-coding-bot/<session_id>/`
persists across deliveries. No known gap here; noted for reference.

### 7f. Credential not yet verified end-to-end for the inv_tool path

**Status:** the respond-mode path (Pattern 1) is confirmed live. The
`inv_tool "telegram.send_message"` + `APXM_RESOLVE_CREDENTIALS` path
(Pattern 2) has not been verified end-to-end from a compiled skill artifact.
The credential resolution code path is present and the proxy endpoint is
implemented in apxm-auth; the integration has not been exercised with the
actual Telegram pack's `tools.toml`.

**Next step:** install the telegram pack, set `APXM_RESOLVE_CREDENTIALS=1`,
store the bot token in apxm-auth as connection id `telegram/main`, compile
a minimal skill with `inv_tool "telegram.send_message"` via
`POST /v1/compile-artifact`, and verify the full proxy path.
