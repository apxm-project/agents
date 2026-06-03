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
- **`read_paths` is an informational grant, not a strict allowlist.**
  `ExecRequest` documents `read_paths`/`write_paths` as "paths the command needs
  to read/write (informational — backend decides enforcement)". bubblewrap
  `--ro-bind / /` over-satisfies the read grant (everything is readable) and
  enforces writes via explicit bind mounts. So a normal request is fully
  satisfied — `validate()` now returns `Ok`, not `Degraded`. Previously it
  flagged non-empty `read_paths` as `Degraded`, which (because both EXC and the
  capability path treat `Degraded` as a hard error) made `bash` **and** EXC fail
  closed under real `bwrap`. Fixed. Tighter per-path *read restriction* (a
  curated base mount instead of binding all of `/`) remains future hardening.
- **The bubblewrap EXECUTE path had a latent mountpoint bug** (never exercised
  before `bwrap` was installed here): it created synthetic mountpoints
  (`/apxm-tmp`, `/apxm-workdir`) *under* the read-only root, which fails with
  "Read-only file system". Rewritten to use `--tmpfs /tmp` and to bind writable
  carve-outs at their real host paths (which exist under the read-only root).
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

## Verification (real `bwrap`, installed + working on the dev host)

`bwrap` is installed and made functional under Ubuntu's userns restriction via
an AppArmor profile granting `userns` (see README → System dependencies).

- Unit/integration: `apxm-runtime` 711, `apxm-server` 259, `apxm-acp` 36,
  `apxm-driver` (incl. two new `bwrap`-gated e2e tests). All pass.
- **Real confinement** (`bubblewrap_confines_writes_to_workdir_only`): a command
  runs, the host root is readable, the working dir is writable, and writes to
  `/etc` are blocked (read-only root).
- **ACP `wrap_command` e2e** (`wrap_command_runs_confined_with_live_stdio`): a
  wrapped child runs under `bwrap` with live stdin/stdout forwarded
  (`got:hello`) and `/etc` writes blocked — proving the ACP path works.
- **Network isolation**: with `needs_network=false`, `--unshare-net` leaves only
  `lo` and outbound connects fail (`NO_NET`); without it, the network is
  reachable.
- **Full stack, live server**: `bash` via an `inv_tool` node through
  `/v1/compile` returns `CONFINED_OK / READ_OK / ETC_BLOCKED` — server → runtime
  → `CapabilitySystem` → `SandboxRegistry` → `BubblewrapSandboxBackend`. The
  previously fail-closed `bash`/EXC paths now run confined.

## Remaining (the larger interface roadmap)

- The full `SessionConfig` / `SpawnRequest` / `SessionHandle` trait evolution
  (see `sandbox-interface.md` on the `investigate/openshell` branch).
- Tighter per-path *read restriction* (curated base mount instead of binding all
  of `/`) for confidentiality, beyond today's read-only-whole-root model.
- Opt-in heavy backings (OpenShell, firecracker, gVisor) behind feature flags.
