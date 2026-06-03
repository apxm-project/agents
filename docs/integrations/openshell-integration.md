# NVIDIA OpenShell ↔ apxm — decision & design

Status: investigation, multi-agent verified · 2026-06-03 · branch `investigate/openshell`
Method: 7 parallel investigators over current code + OpenShell docs/source, cited.

---

## TL;DR

**Marginal value of integrating OpenShell now: 2/5 — fits cleanly, but the gap it
fills is already ~70% covered by apxm's own sandbox.** The honest sequencing:

1. **First (no OpenShell, highest value):** finish wiring apxm's **existing**
   `SandboxBackend` (`Bubblewrap`/`Process`) into the two paths that still run
   with host powers — the `INV`/capability dispatch and the **unsandboxed ACP
   coding-agent spawn** — plus translate apxm-os's inert `sandbox` manifest field
   to a real isolation level. Zero new dependencies; closes the most exploitable
   gaps in a few hundred lines.
2. **Then (optional):** add OpenShell as **one more `SandboxBackend`** behind the
   existing trait, for users who need what bubblewrap can't give: cross-platform
   (macOS/Windows), hot-reloadable network policy, and fleet/multi-tenant
   governance. Drive it via its gateway; feed credentials from apxm-auth; **do
   not wire its inference gateway** (it conflicts with apxm-server's router).

OpenShell is a *much* better-shaped fit than the earlier OpenClaw look (which was
also 2/5) — it plugs into a real, existing trait rather than bolting on a system —
but it is **alpha**, shared-kernel (not a true microVM despite the name), and its
sandbox spin-up is seconds (K3s), which is wrong for apxm's per-turn `EXC`/`INV`
model. So: native first, OpenShell as an opt-in backend later.

---

## 1. What OpenShell is (grounded, 2026)

[NVIDIA OpenShell](https://github.com/NVIDIA/OpenShell) (GTC 2026, part of the
NVIDIA Agent Toolkit, **Apache-2.0**, **alpha / single-player**) is a secure-by-design
runtime that confines each agent in its own sandbox.

- **Control plane = a Gateway daemon** (required): gRPC services (`OpenShell` +
  `Inference`), durable state (SQLite/Postgres), drives Docker/Podman/MicroVM/K8s
  drivers. CLI, **Python SDK** (`SandboxClient`/`SandboxSession.exec`), and a TUI
  all route through it. No Rust SDK — integration is gRPC or CLI/SDK shell-out.
- **Sandbox lifecycle:** `sandbox create … -- <agent>`, `sandbox exec -n <id> --no-tty
  --timeout N -- <cmd>` (captures stdout/stderr/exit — maps to apxm's
  `ExecRequest`→`ExecResult`), `policy set/update`, `provider create`, `delete`.
- **Policy = declarative YAML across 4 domains:** **Filesystem** (`read_only`/
  `read_write` paths, Landlock — locked at creation), **Process** (`run_as_user`,
  seccomp on `SYS_socket` — locked), **Network** (per-endpoint host/port/method/
  path allow+deny rules, OPA/Rego HTTP-CONNECT proxy — **hot-reloadable**),
  **Inference** (gateway-level reroute, not a per-sandbox YAML field).
- **Credentials = "providers":** named bundles injected as env **placeholders**;
  the policy proxy swaps the placeholder for the real secret at egress and
  **fail-closes** (HTTP 500) if it can't — so the raw secret need never sit in the
  sandbox env. Typed providers (`anthropic`/`codex`/`copilot`/`openai`/…) inject the
  conventional env var. Auto-discovers creds for claude/codex/opencode/copilot.
- **Inference reroute:** inside a sandbox, `https://inference.local/v1/{chat/completions,
  responses,messages}` is intercepted; the router strips the caller key, injects
  the backend provider, rewrites `model`, forwards to a configured endpoint —
  **which can be apxm-server** (`provider create --type openai --config
  OPENAI_BASE_URL=http://host.openshell.internal:<port>/v1`).
- **Requirements:** Docker 28+/Podman 5/MicroVM/K8s. **GPU is opt-in** (`--gpu`,
  NVIDIA-only); not required for CPU sandboxes. Hosts macOS/Windows-WSL2/Linux.
- **Reality check:** "MicroVM" is largely eBPF/seccomp/Landlock **shared-kernel**
  hardening (weaker than Firecracker/gVisor for kernel-escape); sandbox creation
  is **seconds** (K3s), not ms.

Sources: `github.com/NVIDIA/OpenShell`; `docs.nvidia.com/openshell/*` (how-it-works,
policy-schema, providers, inference-routing); `developer.nvidia.com/blog/run-autonomous-self-evolving-agents-more-safely-with-nvidia-openshell`.

## 2. apxm's sandbox reality (verified on `main`)

Better-built than the OpenClaw-era notes implied:

- **`SandboxBackend` trait + `SandboxRegistry`** (`apxm-runtime/src/sandbox/`):
  `capabilities / is_available / validate / create_session / execute(ExecRequest)
  → ExecResult / destroy_session`. `ExecRequest` already carries
  `read_paths`/`write_paths`/`needs_network`/`needs_process_spawn`/`env`/`timeout` —
  i.e. the inputs to an OpenShell policy.
- **Two real backends, registered:** `ProcessSandboxBackend` (policy-only fallback)
  + `BubblewrapSandboxBackend` (Linux namespaces, <50 ms) via
  `configure_sandbox_registry()` (`apxm-driver/.../sandbox.rs:155`); wired with
  `runtime.set_sandbox_registry()`.
- **`EXC` op routes through the registry** (`exc.rs:46`, hard-errors with no backend).
- **apxm's policy is RICHER than OpenShell in places:** capability-granular write
  boundary enforced at **every** `INV_TOOL` (incl. nested via no-widen
  `side_effect_policy` propagation, `call_skill.rs`), the visible-set/`imports`
  call-graph containment, read/write capability path allowlists, bash denylist,
  `guard_url_ssrf`, and ASK tool-exposure being read-only-by-default.

**The gaps OpenShell (or finishing bubblewrap) would close:**
- `INV`/most capabilities (`read`/`write`/`http`) don't route through the sandbox —
  only `bash`/`EXC`/`UserTool` do; bash even has a dual direct-exec path.
- **ACP coding-agent spawn is fully unsandboxed** (`apxm-acp/session.rs`:
  `tokio::process::Command::new(...).spawn()`); the reverse-handler gives the agent
  host read/write/bash; creds come from the spawner's ambient env.
- apxm-os `Isolation::{Task,Subprocess}` + `Sandbox::{None,Bubblewrap,Docker,Wasm}`
  are parsed but **never branched on** (agents run as in-process tokio tasks).
- No OS enforcement on the `kind=http` capability (no SSRF); no egress *allowlist*;
  no cross-platform (bubblewrap is Linux-only); no hot-reload policy.

## 3. Integration design — OpenShell as one `SandboxBackend`

If/when pursued, OpenShell slots in behind the existing trait — **no trait/op/wire
changes**:

```
apxm-sandbox-openshell (new crate / driver module) : impl SandboxBackend
  is_available()    -> gateway reachable + image present
  create_session()  -> openshell sandbox create (gRPC); store sandbox id in ctx.inner
  execute(req)      -> derive YAML policy from req (read_paths→filesystem.read_write,
                       needs_network→network, needs_process_spawn→process);
                       inject req.env as a provider bundle; `sandbox exec --no-tty
                       --timeout` ; capture stdout/stderr/exit -> ExecResult
  destroy_session() -> openshell sandbox delete
registered in configure_sandbox_registry() behind a config/feature flag,
IsolationLevel::Container (or Hypervisor if MicroVM driver).
```

**Policy mapping (apxm primitive → OpenShell domain):**

| OpenShell domain | apxm source |
|---|---|
| Filesystem `read_only`/`read_write` | `ExecRequest.read_paths`/`write_paths`; `ReadConfig`/`WriteConfig` allowlists |
| Process (user, command policy) | `needs_process_spawn`; `BashConfig` denylist/allowlist; the write boundary / `side_effect_policy` |
| Network (endpoint allow/deny) | `guard_url_ssrf` denylist as base; `provider.call` → apxm-auth `/proxy` host as the allowlisted egress |
| Inference (reroute) | **apxm-server** as the controlled backend (OpenAI-compat) — *optional, see below* |

**Credentials — apxm-auth stays sole custodian.** Add `env_var` to `ProviderRecipe`
and a `GET /v1/connections/{id}/env-bundle` endpoint; at sandbox creation the
caller resolves the bundle from apxm-auth and hands it to OpenShell as a provider
(env-placeholder injection). Better than today's `agents.toml` ambient-env hand-off
(rotation, OAuth refresh, audit, per-sandbox scope). For apxm-controlled calls,
prefer apxm-auth `/proxy` (secret never enters the sandbox at all); env-injection
is the path for third-party coding CLIs that read keys from env.

**ACP coding agents — the highest-value sandboxing.** Route `AcpSession::spawn`
through the `SandboxBackend` (build an `ExecRequest` for the agent command), so
claude/codex run inside the sandbox with policy-bounded filesystem/network and
apxm-auth-injected creds — instead of full host powers. This works with **bubblewrap
today**; OpenShell only adds cross-platform + hot-reload.

**Inference — do NOT wire OpenShell's gateway by default.** apxm-server already owns
model routing (`/v1/backends`, ModelRouter); inserting OpenShell's `inference.local`
router between agent and backend is a second routing plane = a conflict. Only point
`inference.local` at apxm-server for *sandboxed third-party coding agents* that you
explicitly want governed at the network layer.

**apxm-os wiring (minimal, no dep-break):** add the mapping `sandbox = "docker"/
"openshell"` → `min_isolation` on the `SkillExecuteRequest` to apxm-server; the
server's registry then selects the OpenShell backend. apxm-os keeps zero apxm-cargo
deps; the backend lives in apxm-runtime/driver.

## 4. Risks
- **Alpha / single-player**; rapid breaking releases; K3s/gateway operational weight.
- **Shared-kernel** hardening, not a true microVM — weaker isolation than the name implies.
- **Seconds-scale sandbox creation** — unsuitable for per-turn `EXC`/`INV`; needs
  session reuse, not create-per-call.
- **No Rust SDK** → drive via gRPC or shell-out (a process supervising a process).
- **Inference-gateway conflict** with apxm-server (see above).
- **NVIDIA coupling** only for `--gpu` (AMD/Apple excluded there); CPU path is vendor-neutral.
- **Lock-in** to NVIDIA Agent Toolkit's evolution; mitigated by keeping it one
  pluggable backend behind apxm's own trait.

## 5. Recommendation & phased plan
- **P0 — finish the native sandbox (no OpenShell, do now):** route `INV`/capability
  dispatch through `SandboxRegistry` (mirror `exc.rs`); sandbox `AcpSession::spawn`
  via the backend; map apxm-os `sandbox` field → `min_isolation`. Closes the
  unsandboxed-bash/write + unsandboxed-coding-agent holes with bubblewrap, zero deps.
- **P1 — OpenShell as an opt-in backend:** `OpenShellBackend: SandboxBackend` behind
  a config flag, gateway-driven, apxm-auth env-bundle for creds, **inference gateway
  off**. For cross-platform, hot-reload network policy, and the fleet/multi-tenant
  future.
- **Revisit/justify OpenShell when:** apxm-os goes multi-tenant (distinct trust
  boundaries per agent); macOS/Windows isolation is needed; or a user's threat model
  exceeds bubblewrap. Until then it is additive, not load-bearing.

## Provenance
7 investigators: SandboxBackend seam, ACP spawn, OpenShell programmatic surface,
policy mapping, apxm-auth credential plane, apxm-os isolation wiring, strategic
referee. Verified on `main`: bubblewrap/process backends + registry + `EXC` routing
+ `sandbox_e2e.rs` exist; `INV`/ACP/os-os gaps confirmed.
