# ACP sandbox seam + sandbox-routing corrections

Implements the highest-value, verifiable slice of the backend-agnostic sandbox
interface: confining the long-running ACP coding-agent surfaces, plus
corrections to claims the design draft and an integration test made about the
existing capability path.

## What the investigation found (corrects earlier assumptions)

- **The capability / `INV_TOOL` path is NOT an unsandboxed bypass.**
  `CapabilitySystem::invoke_with_timeout` already routes any capability whose
  `to_exec_request()` returns `Some` through
  `SandboxRegistry::select_for_request` → `create_session` → `execute` →
  `destroy_session`. It falls back to direct `execute()` only when a capability
  declares no `ExecRequest`. Process-spawning capabilities (`bash`, user tools)
  therefore run *inside* the sandbox or **fail closed** — they are never run
  unconfined.
- **`bash` currently fails closed on hosts without `bwrap`.** Its `ExecRequest`
  asks for `OsLevel`; the policy-only `ProcessSandboxBackend` is `PolicyOnly`
  (filtered out), and a degraded backend is treated as a hard error. This is a
  usability gap, not an insecurity.
- **The draft's "minimal fix" (drop the bubblewrap read-allowlist warning) would
  be unsafe.** The bubblewrap backend `--ro-bind / /` exposes the *entire* root
  read-only, so it genuinely does not honor a per-path read allowlist. The
  `Degraded` warning is truthful; removing it would claim enforcement that does
  not exist. Honoring read allowlists precisely is future work (curated base
  mount instead of binding all of `/`).
- **The real residual holes are the ACP surfaces**, which spawn directly on the
  host with no sandbox involvement:
  - `AcpSession::spawn` — the coding-agent subprocess itself.
  - `TerminalManager::create` (reverse `terminal/create`) — arbitrary commands
    the agent asks the client to run. Confining the agent while leaving this
    open would be pointless.
- The integration test `test_gap_sandbox_not_called_by_inv_handler` was **stale**
  — it asserted the gap was open without invoking anything. Replaced with
  `test_registry_selected_backend_is_executed`, which exercises the
  select→execute sequence the capability path performs.

## What shipped

A `wrap_command` seam on the existing `SandboxBackend` trait — no `SessionConfig`
/ `SpawnRequest` refactor required:

- `SandboxBackend::wrap_command(program, args, cwd, needs_network)` rewrites a
  command into a confined equivalent. Default impl is a no-op (process-policy /
  remote backends cannot confine a caller-owned spawn). Isolation wrappers like
  `bwrap` forward stdin/stdout/stderr transparently, so the caller's pipe
  handling — and `StdioTransport` — are untouched.
- `BubblewrapSandboxBackend::wrap_command` builds a session-free `bwrap` argv:
  read-only root, ephemeral `/tmp`, user+pid namespaces, network kept only when
  needed (coding agents reach the model gateway), writable cwd bound in place.
  Pure argv builder is unit-tested without `bwrap`.
- ACP wiring (opt-in, **default off**): `AcpAgentProfile.sandbox` gates it;
  `AcpSession::spawn`, `CapabilityReverseHandler`, and `TerminalManager` carry an
  optional backend and wrap before spawning. The driver selects a capable
  backend only when a profile opts in, and **fails closed** if one opts in but no
  backend can confine a long-running child.

Default-off means existing deployments spawn byte-identically; confinement
engages only on a `bwrap`-equipped host with a profile that requests it.

## Verification

- `apxm-runtime` 711 lib tests, `apxm-server` 259, `apxm-acp` 36, `apxm-driver`
  71 — all pass. New: bubblewrap wrap argv tests, the replaced routing test.
- Rebuilt server starts healthy with the new agent-registry wiring; capabilities
  register; `/v1/compile` validates and executes a graph end-to-end
  (`executed_nodes:2, failed_nodes:0`).
- Real `bwrap` confinement of an agent is not exercisable on the dev host
  (`bwrap` absent; the gateway `thinking.type` quirk blocks `claude`, only
  `codex` spawns), so the argv is unit-tested rather than run.

## Remaining (the larger interface roadmap)

- The full `SessionConfig` / `SpawnRequest` / `SessionHandle` trait evolution
  (see `sandbox-interface.md` on the `investigate/openshell` branch).
- Make the bubblewrap backend honor per-path read allowlists (base mount instead
  of `--ro-bind / /`) so `bash` validates `Ok` and runs confined when `bwrap` is
  present.
- Opt-in heavy backings (OpenShell, firecracker, gVisor) behind feature flags.
