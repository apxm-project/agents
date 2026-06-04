# Stack connectivity: studio + os → the execution/sandbox backend

How apxm-studio and apxm-os connect to apxm-server (the execution backend that
owns the sandbox), verified by a three-way code audit. The short version: both
**already route execution through apxm-server over HTTP**, so the server-side
sandbox applies to their work. This documents the seams and the contract that
makes the sandbox intent honest.

## Connection map

### apxm-studio → apxm-server
```
browser SPA → studio Rust backend (proxy, :18802) → apxm-server (:18800)
```
- Canvas is lowered to AIR **in studio** (Rust `lower` → Python frontend → `.air`
  MLIR), then `POST /v1/execute/stream` (fallback `/v1/execute`).
- Write-boundary consent is wired: a canvas carries `granted_capabilities`, sent
  as `admit_capabilities`; the server's admission gate returns 409 → studio
  shows a consent modal → grant → re-run.
- Backend/model pickers read `GET /v1/backends` (unified registry — no
  re-derivation).
- Base URL: `apxm_ais::defaults::DEFAULT_SERVER_URL` (`http://127.0.0.1:18800`),
  overridable by `--server` / `APXM_SERVER_URL`.

### apxm-os → apxm-server
```
trigger (cron/watch/webhook/channel/a2a) → CueEvent → Dispatcher
  → ApxmServerClient → POST /v1/skills/{id}/execute
```
- apxm-os has **no apxm crate dependency** — it talks to the backend purely over
  HTTP (clean boundary; nothing runs unsandboxed in-process).
- `target_server` per agent manifest (default `http://127.0.0.1:18800`).

## The sandbox boundary (who confines what)

The sandbox lives **inside apxm-server** (the `SandboxRegistry`: bubblewrap when
available, else policy-only process). It confines EXC and process-spawning
capabilities of whatever AIR/skill runs. studio and os do not run tools
themselves — they hand work to apxm-server, so confinement is automatic.

Two things cross the wire to control it:

1. **`admit_capabilities`** (studio → `/v1/execute*`): the write-boundary grant.
   A non-read-only, non-sandboxed capability runs only if granted. Wired in
   studio; **not** carried by os's `/v1/skills/{id}/execute` (os skills should be
   read-only or sandboxed — write admission on that path is future work).

2. **`sandbox_hint`** (os → `/v1/skills/{id}/execute`): the minimum isolation an
   agent requires, from its manifest `sandbox` field (`none | bubblewrap |
   docker | wasm`). The server maps it to an `IsolationLevel` and **fails closed**
   if no registered backend satisfies it — so an agent that declares
   `sandbox = bubblewrap` never silently runs less confined. Before this, the
   manifest field was parsed but never sent or honored (a no-op).

   ```
   none       → no requirement
   bubblewrap → Container   docker → Container   wasm → Wasm
   ```
   Verified e2e: `wasm` on a bwrap-only host → 400 fail-closed; `bubblewrap` →
   passes; unknown value → 400.

## Configuration & auth

- **Exec backend URL** is consistent: the `apxm_ais::defaults` constants
  (`DEFAULT_SERVER_URL`, `DEFAULT_OS_URL`) are the single source; studio reads
  them via clap (`APXM_SERVER_URL` / `APXM_OS_URL`), os via per-agent
  `target_server`. `scripts/stack.env` in studio is the deployment source of
  truth.
- **LLM backend registry** is unified behind `GET /v1/backends` (no divergent
  lists).
- **Auth**: apxm-server's `require_auth` is **off by default**. studio and os
  attach a bearer to apxm-auth calls but not (yet) to apxm-server calls, so
  enabling `require_auth` would require wiring a bearer on those paths. Tracked
  below.

## Known items (status, not silently dropped)

- **studio `StudioConfig.apxm_server_url` is not loaded at startup** — the live
  URL comes from env/CLI; the persisted setting is currently inert. Fixing it is
  a precedence decision (should a saved setting override `APXM_SERVER_URL`?) left
  to the owner. The connection itself works via env/CLI.
- **studio ACP "Coding Agent" node has no per-node `sandbox` toggle.** Agent
  sandboxing is set at the **profile** level (`apxm agent add --sandbox`), which
  studio selects; a per-node override would need an AIR `Op::Agent` attribute +
  runtime honoring it.
- **studio memory nodes** (`MemoryWrite` → `memory.store_fact`) are not included
  in studio's write-consent scan, which only covers `NodeKind::Tool`. Whether
  this is a gap depends on `memory.store_fact`'s registered `read_only` status —
  confirm before changing the gate.
- **os → server has no bearer** and no `admit_capabilities` path (read-only/
  sandboxed skills only on that route).
