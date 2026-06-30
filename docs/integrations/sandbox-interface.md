# apxm sandbox interface — backend-agnostic design spec

> **SUPERSEDED IN PART — read with `docs/integrations/sandbox-acp-seam.md` on
> `main`.** A later code-level investigation corrected several premises here:
> the capability/`INV` path is **not** an unsandboxed bypass (it already routes
> through `SandboxRegistry` and fails closed); `read_paths` is an *informational
> grant*, so the "remove the read-allowlist warning" idea was wrong (the real
> fix was `validate()` returning `Ok`); the genuine holes were the ACP
> spawn/terminal surfaces, now closed by a `wrap_command` seam. The full
> `SessionConfig`/`SpawnRequest` trait evolution below remains valid future work.

Status: design, multi-agent e2e-verified · 2026-06-03 · branch `investigate/openshell`
Method: 13-agent workflow — investigate (current trait + consumers + candidate
backends + policy + transport/creds) → design → adversarial verify e2e against each
backend (process/bubblewrap/OpenShell/firecracker/gVisor/Docker) and each consumer
(EXC, INV/capabilities, ACP coding-agent, apxm-os agents) → reconcile.

> Principle: **apxm owns the interface; every sandbox is a swappable backing.** The
> existing `SandboxBackend` trait + `SandboxRegistry` is the seam; this spec evolves it
> to host process / bubblewrap (now) and OpenShell / firecracker / gVisor / Docker
> (opt-in) without vendor lock-in. The adversarial pass below changed the draft (see end).

---
# apxm Sandbox Interface — Final Reconciled Spec

## 0. Design tenets

1. **apxm owns the interface; backends are swappable backings.** The trait lives in `crates/runtime/engine/src/sandbox/`. Process / Bubblewrap / OpenShell / Firecracker / gVisor / Docker are implementations registered by the host. No vendor type appears on any core struct; backend-specific config rides `SessionPolicy.extra` (opaque `serde_json::Value`).
2. **One-shot stays the hot path and stays source-compatible.** `execute(ctx, ExecRequest) -> ExecResult` is unchanged in shape (EXC + INV). Process (<1 ms) and Bubblewrap (<50 ms warm) never honor heavy fields.
3. **Add, don't bend.** Long-running stdio is a new `spawn` method returning `SessionHandle`. Hot-reload is `update_policy`. Liveness is `is_alive`. Session-level validation is `validate_config`. All defaulted so existing backends compile unchanged.
4. **Capability negotiation, not lowest-common-denominator.** Backends declare `SandboxCapabilities`; the registry hard-filters on platform + hard criteria, then per-request `validate` returns structured `Degraded { gaps }` naming every unenforceable field (no-silent-fallback contract).
5. **Session lifetime is explicit and owned by the consumer.** `SandboxLifetime` distinguishes per-call / per-graph-or-agent. A concrete `SandboxSessionGuard` RAII type — not prose — guarantees teardown across tokio cancellation.
6. **apxm-auth is the sole credential custodian.** Plaintext lives only in `Zeroizing` env maps injected immediately before dispatch, or never enters the request at all via proxy `CredentialRef`s. Session-scoped credential refs live on `SessionConfig`.

---

## 1. The trait

Existing three method signatures (`capabilities`, `is_available`, `validate`) keep their shape; `execute` / `destroy_session` are unchanged; `create_session` gains a `SessionConfig` argument; six methods are **added with defaults**. The `Debug` supertrait is **NOT** added (resolves the OpenShell-RC4 / Bubblewrap-Gap test-double break — registry debug output already uses `capabilities().name`, not `{:?}` on the backend).

```rust
#[async_trait]
pub trait SandboxBackend: Send + Sync {
    // ---- introspection (unchanged shapes) ----
    fn capabilities(&self) -> SandboxCapabilities;

    /// Cheap, synchronous, MUST be memoized. Expensive probes (bwrap --version,
    /// /dev/kvm, gateway reachability) cached behind a OnceLock/AtomicBool.
    fn is_available(&self) -> bool;

    /// Per-call ExecRequest compatibility. Returns Ok / Degraded{gaps} /
    /// Unsupported{reason}. MUST name every ignored field in `gaps`.
    fn validate(&self, request: &ExecRequest) -> ValidationResult;

    // ---- NEW: session-level validation (default Ok) ----
    /// Validate session-scoped fields BEFORE the (possibly expensive) create.
    /// Catches missing mandatory `extra` keys (Firecracker kernel/rootfs),
    /// unreachable credential bundles, unenforceable resource_limits. The
    /// registry runs this during selection so session-level Degraded/Unsupported
    /// is visible to the no-silent-fallback contract, not buried in a runtime
    /// SessionError after a 3–10 s boot.
    fn validate_config(&self, _config: &SessionConfig) -> ValidationResult {
        ValidationResult::Ok
    }

    /// Mandatory `SessionPolicy.extra` keys this backend needs (Firecracker:
    /// ["kernel_image","rootfs"]). Registry fails fast before create.
    fn required_extra_keys(&self) -> &'static [&'static str] { &[] }

    // ---- session lifecycle ----
    /// Create ONCE per logical scope (per-call / per-graph / per-agent — see
    /// SessionConfig.lifetime). May be expensive. Stores backend-private state
    /// in ctx.inner. Honors config.create_timeout for boot-bounded backends.
    async fn create_session(&self, config: SessionConfig)
        -> Result<SandboxContext, SandboxError>;

    /// Run ONE command to completion, fully buffered. Hot path for EXC + INV.
    async fn execute(&self, ctx: &SandboxContext, request: ExecRequest)
        -> Result<ExecResult, SandboxError>;

    /// Tear down. MUST be idempotent and best-effort: attempt every cleanup step
    /// independently (do NOT short-circuit on first error); aggregate failures
    /// into the returned SandboxError. Invoked through SandboxSessionGuard so the
    /// CALL is guaranteed even on panic/cancel (the backend owns whether work
    /// completes; leaked VM/container state is the backend's recovery problem,
    /// but the guard removes the "destroy was never called" failure mode).
    async fn destroy_session(&self, ctx: SandboxContext) -> Result<(), SandboxError>;

    // ---- NEW: liveness (default = sandbox IS the child → Ok(true)) ----
    /// Is the session's underlying isolation still up? For Process/Bubblewrap the
    /// sandbox dies with the child, so the default Ok(true) is correct (the caller
    /// learns of exit via the child handle). Docker/OpenShell/Firecracker implement
    /// a real probe (docker inspect / gRPC Ping / vsock heartbeat) so the apxm-os
    /// supervisor can distinguish "agent exited" from "sandbox crashed".
    async fn is_alive(&self, _ctx: &SandboxContext) -> Result<bool, SandboxError> {
        Ok(true)
    }

    // ---- NEW: long-running stdio child (default = Unsupported) ----
    /// Launch a long-lived child and return live async stdio + a kill/poll control
    /// handle. For ACP coding agents and apxm-os Subprocess agents. One spawn per
    /// session; reverse callbacks ride the same child.
    async fn spawn(&self, _ctx: &SandboxContext, _request: SpawnRequest)
        -> Result<SessionHandle, SandboxError> {
        Err(SandboxError::Unsupported("no long-running attached sessions".into()))
    }

    // ---- NEW: hot-reload of mutable policy (default = Unsupported) ----
    /// Push new values for hot-reloadable fields onto a running session. Safe to
    /// call CONCURRENTLY with execute (backends that cannot must serialize via
    /// inner state; the trait adds no lock). OpenShell honors it; static backends
    /// keep the default error.
    async fn update_policy(&self, _ctx: &SandboxContext, _delta: PolicyDelta)
        -> Result<(), SandboxError> {
        Err(SandboxError::Unsupported("no hot-reload policy".into()))
    }

    // ---- NEW: feature gate (default false) ----
    fn supports_streaming(&self) -> bool { false }
}
```

Why the placement:
- `execute` is NOT generalized to streaming. `ExecResult` is by-value buffered; streaming needs owned async readers → `spawn`/`SessionHandle`.
- `validate_config` + `required_extra_keys` close the session-level no-silent-fallback hole flagged by every heavy-backend verdict (OpenShell credential_bundle reachability, Firecracker kernel/rootfs, resource_limits on cgroup-less backends). `validate(&ExecRequest)` structurally cannot see `SessionConfig`.
- `is_alive` closes the apxm-os supervisor liveness gap.
- Defaults mean Process / Bubblewrap / `DefaultBackend` need only the `create_session(config)` arg change.

---

## 2. Key types

### 2.1 IsolationLevel (unchanged ordinals) + a sub-capability for true KVM

`None=0 < PolicyOnly=1 < OsLevel=2 < Container=3 < Hypervisor=4 < Wasm=5 < Remote=6`. Kept `Ord + Display + serde`. Because gVisor (user-space kernel) and Firecracker (true KVM) both map to `Hypervisor`, a request that needs real kernel isolation cannot distinguish them by ordinal alone. Resolution: **do not split the enum** (it is an integer ordinal used for ordering); add `supports_kvm_isolation: bool` to capabilities and `require_kvm_isolation: bool` to `SelectionCriteria`.

| Backend | IsolationLevel | kvm |
|---|---|---|
| Process | PolicyOnly | false |
| Bubblewrap | Container | false |
| Docker / Podman | Container | false |
| OpenShell (shared-kernel eBPF/seccomp/Landlock) | Container | false |
| gVisor (runsc, user-space kernel) | Hypervisor | false |
| Firecracker / Cloud-Hypervisor (true KVM) | Hypervisor | true |

### 2.2 SandboxCapabilities (additive)

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxCapabilities {
    pub isolation_level: IsolationLevel,
    pub supports_filesystem_restriction: bool,
    pub supports_network_restriction: bool,
    pub supports_syscall_filtering: bool,
    pub supports_resource_limits: bool,
    // NEW:
    pub supports_persistent_session: bool, // false = create-per-exec (Process/bwrap)
    pub supports_hot_reload_policy: bool,   // update_policy honored (OpenShell)
    pub supports_credential_bundle: bool,   // session-scoped CredentialRefs resolved
    pub supports_kvm_isolation: bool,       // true KVM vs user-space kernel
    pub supported_platforms: Vec<Platform>, // registry pre-filter BEFORE is_available
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Platform { Linux, MacOs, Windows }
```

`supports_streaming` is a trait method (gates a method, may be config-derived), not a field.

### 2.3 ExecRequest (one-shot; 2 additive default-inert fields)

Existing fields kept exactly. Added:
```rust
    /// Proxy-injected creds resolved at egress by capable backends; plaintext
    /// never lands in `env`. Honored only when supports_credential_bundle, else Degraded.
    pub cred_refs: Vec<CredentialRef>,
    /// Per-call egress refinement consumed by OpenShell/Docker/gVisor; Degraded by
    /// Process/Bubblewrap (which only honor `needs_network`).
    pub network_policy: Option<NetworkPolicy>,
```
`Default` yields `cred_refs: vec![]`, `network_policy: None`. `env` carries only resolved short-lived secrets, wrapped `Zeroizing` at the dispatch layer and zeroized after the one-shot child exits. No vendor fields.

### 2.4 ExecResult (unchanged)

`{ success, exit_code, stdout, stderr, duration, timed_out }`. Deliberately not extended for streaming.

### 2.5 SpawnRequest + SessionHandle + ChildControl (NEW — long-running stdio)

```rust
#[derive(Debug)]
pub struct SpawnRequest {
    pub min_isolation: IsolationLevel,
    pub program: String,
    pub args: Vec<String>,
    pub working_dir: Option<PathBuf>,
    /// Long-lived secrets. Zeroizing<HashMap>; the zeroization guarantee is tied
    /// to SessionHandle drop (child may run for hours), NOT "after child exits".
    pub env: Zeroizing<HashMap<String, String>>,
    pub read_paths: Vec<PathBuf>,
    pub write_paths: Vec<PathBuf>,
    pub needs_network: bool,
    pub network_policy: Option<NetworkPolicy>,
    pub cred_refs: Vec<CredentialRef>,
    pub origin_op: Option<String>,
}

/// Live handle to a spawned long-running child. Owns stdio as TRAIT OBJECTS so any
/// backend (local process, vsock guest-agent, docker exec) is uniform.
pub struct SessionHandle {
    pub stdin: Box<dyn tokio::io::AsyncWrite + Send + Unpin>,
    pub stdout: Box<dyn tokio::io::AsyncRead + Send + Unpin>,
    pub stderr: Box<dyn tokio::io::AsyncRead + Send + Unpin>,
    control: Box<dyn ChildControl + Send + Sync>,
}

#[async_trait]
pub trait ChildControl {
    fn try_wait(&mut self) -> Result<Option<ExitStatus>, SandboxError>;
    async fn wait(&mut self) -> Result<ExitStatus, SandboxError>;
    /// Local processes: POSIX signal. Hypervisor backends: vsock message to the
    /// guest agent (requires a live agent). See `shutdown` for the VM-level path.
    fn signal(&mut self, sig: Signal) -> Result<(), SandboxError>;
    /// Sever stdin only (EOF to the child) WITHOUT signaling — lets ACP keep its
    /// drop-stdin → grace → signal close sequence.
    async fn close_stdin(&mut self) -> Result<(), SandboxError>;
    /// VM/container-level teardown distinct from a guest-process signal (Firecracker
    /// API poweroff, docker stop). Default delegates to signal(Kill).
    async fn shutdown(&mut self) -> Result<(), SandboxError> { self.signal(Signal::Kill) }
    /// Best-effort pid for Unix SIGTERM parity with existing code; None for remote.
    fn id(&self) -> Option<u32> { None }
}

#[derive(Debug, Clone, Copy)] pub enum Signal { Term, Kill }

#[derive(Debug, Clone)]
pub struct ExitStatus {
    pub code: Option<i32>,
    pub signaled: bool,
    /// Sandbox-level exit context for the supervisor's restart policy.
    pub sandbox_exit: Option<SandboxExitReason>,
}

#[derive(Debug, Clone)]
pub enum SandboxExitReason { Normal, OomKilled, SandboxDeleted, TransportError(String) }

impl SessionHandle {
    pub fn try_wait(&mut self) -> Result<Option<ExitStatus>, SandboxError> { self.control.try_wait() }
    pub async fn wait(&mut self) -> Result<ExitStatus, SandboxError> { self.control.wait().await }
    pub fn signal(&mut self, s: Signal) -> Result<(), SandboxError> { self.control.signal(s) }
    pub async fn close_stdin(&mut self) -> Result<(), SandboxError> { self.control.close_stdin().await }
    pub async fn shutdown(&mut self) -> Result<(), SandboxError> { self.control.shutdown().await }
    pub fn pid(&self) -> Option<u32> { self.control.id() }
}
```

ACP mapping (verified against `protocol.rs:157` `read_response`, which interleaves read + write on a single `&mut self` and never reads/writes concurrently): a **single struct owning both halves is sufficient — no split needed**. The only protocol change is widening `StdioTransport`'s fields from concrete `ChildStdin`/`ChildStdout` (protocol.rs:58-59,64) to the trait-object types above. `close_stdin` + `signal` reproduce the existing drop-transport → grace → SIGTERM → SIGKILL sequence (session.rs:302-333); `pid()` preserves the Unix `child.id()` SIGTERM path (session.rs:316).

### 2.6 SessionConfig (NEW — argument to create_session)

```rust
#[derive(Debug, Default)]
pub struct SessionConfig {
    pub policy: SessionPolicy,
    pub resource_limits: Option<ResourceLimits>,
    /// Session-scoped credential refs (OpenShell providers created at create-time;
    /// remote backends). Replaces a single bundle id — the granularity heavy
    /// backends actually need. Resolved via apxm-auth then dropped; never serialized.
    pub credential_refs: Vec<CredentialRef>,
    /// Max time to wait for create (VM boot, gRPC CreateSandbox). None = caller's
    /// outer timeout. Surfaces SandboxError::Timeout instead of hanging a graph.
    pub create_timeout: Option<Duration>,
    /// Max time to wait for guest-agent readiness AFTER boot, distinct from any
    /// per-command ExecRequest.timeout. Firecracker/microVM use this.
    pub ready_timeout: Option<Duration>,
    /// Who owns the session lifetime — drives where the consumer holds the ctx.
    pub lifetime: SandboxLifetime,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SandboxLifetime {
    /// Create+destroy around a single execute (Process/bwrap one-shot default).
    #[default] PerCall,
    /// One session reused across a graph run / agent-task invocation.
    Scope,
    /// One session that SURVIVES agent restarts (held by the supervisor, re-spawn
    /// into the surviving ctx). For Docker/OpenShell/Firecracker long-lived agents.
    Supervisor,
}

#[derive(Debug, Clone, Default)]
pub struct ResourceLimits {
    pub max_cpu_ms: Option<u64>,
    pub max_memory_bytes: Option<u64>,
    pub max_pids: Option<u32>,
    pub max_disk_bytes: Option<u64>,
}
```

`Default` keeps the one-line migration (`SessionConfig::default()`). The Firecracker "Default hides mandatory fields" concern is resolved structurally: mandatory `extra` keys are declared via `required_extra_keys()` and checked by the registry during `validate_config`, so a defaulted config that omits kernel/rootfs is rejected at **selection**, before the boot attempt.

### 2.7 SessionPolicy

```rust
#[derive(Debug, Clone, Default)]
pub struct SessionPolicy {
    pub filesystem: FsPolicy,        // STATIC
    pub process: ProcPolicy,         // STATIC
    pub network: NetworkPolicy,      // HOT-RELOADABLE
    pub inference: InferencePolicy,
    pub credentials: CredPolicy,     // STATIC (resolution at create)
    /// Path translation for backends that cannot bind-mount host paths at the same
    /// path (microVMs). Empty for containers (implicit same-path bind). A microVM
    /// backend that receives read_paths/write_paths with NO mapping returns Degraded
    /// naming them (so EXC rejects rather than silently mis-locating files).
    pub host_to_guest_paths: Vec<(PathBuf, PathBuf)>,
    /// Vendor escape hatch (OpenShell YAML, Firecracker kernel/rootfs, GPU VFIO id,
    /// inference intercept endpoint). Opaque to core; capable backends read it.
    pub extra: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Default)]
pub struct FsPolicy {              // STATIC
    pub read_paths: Option<Vec<PathBuf>>,
    pub write_paths: Option<Vec<PathBuf>>,
    pub blocked_paths: Vec<PathBuf>,
    pub base_directory: Option<PathBuf>,
    pub max_file_size_bytes: Option<u64>,
    pub allowed_extensions: Option<Vec<String>>,
    pub blocked_extensions: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ProcPolicy {           // STATIC
    pub run_as_user: Option<String>,
    pub allowed_commands: Option<Vec<String>>,
    pub allowed_interpreters: Option<Vec<String>>,
    pub needs_process_spawn: bool,
    pub syscall_filter_profile: SyscallProfile, // None | DefaultSeccomp | Strict
}

#[derive(Debug, Clone, Default)]
pub struct NetworkPolicy {         // HOT-RELOADABLE
    pub egress_allow: Option<Vec<String>>, // None = deny-all-private
    pub egress_deny: Vec<String>,
    pub allowed_schemes: Vec<String>,      // default [http, https]
    pub ssrf_guard_enabled: bool,          // default true; mirrors guard_url_ssrf
}

#[derive(Debug, Clone, Default)]
pub struct InferencePolicy {
    pub backend: Option<String>,             // STATIC — maps to apxm-server `backend` attr
    pub allowed_models: Option<Vec<String>>, // HOT-RELOADABLE
    pub intercept_enabled: bool,             // DEFAULT FALSE
    /// Typed endpoint the sandboxed agent's inference is rerouted to (apxm-server's
    /// OpenAI-compat address). REQUIRED when intercept_enabled; supplied by the
    /// CALLER (driver/ACP), since the sandbox crate cannot derive the server address.
    /// Typed, not buried in `extra`.
    pub intercept_endpoint: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct CredPolicy {           // STATIC
    pub env_passthrough: Vec<String>,
    pub env_blocked: Vec<String>,
    pub env_overrides: HashMap<String, String>,
}

#[derive(Debug, Clone, Default)]
pub struct PolicyDelta {          // only hot-reloadable fields
    pub network: Option<NetworkPolicy>,
    pub allowed_models: Option<Vec<String>>,
    pub max_cpu_ms: Option<u64>,
    pub max_memory_bytes: Option<u64>,
}
```

`IsolationLevel` is NOT in the policy (it is a selection signal). `base_directory` / `run_as_user` / `syscall_filter_profile` / `inference.backend` are static (mutating them mid-session changes a running perimeter). Only `network.*`, `allowed_models`, and cpu/mem limits are hot-reloadable.

### 2.8 Credentials

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CredentialRef {
    pub connection_id: String,
    /// Env var the resolved secret binds to (env path) OR the placeholder the egress
    /// proxy swaps (proxy path).
    pub env_var: String,
}
```
Two modes by capability:
- **Env injection (all backends):** dispatch layer calls apxm-auth `GET /v1/connections/env-bundle?ids=…` (bearer-gated, `is_secret_slot`-guarded, audited) → `Zeroizing<HashMap>` placed in `env`, zeroized after one-shot exit / `SessionHandle` drop.
- **Proxy injection (OpenShell, remote):** `cred_refs` passed through; backend creates an OpenShell *provider* (placeholder) and the OPA egress proxy swaps at egress, fail-closing. Plaintext never enters the sandbox env. Gated by `supports_credential_bundle`.

Session-scoped refs go on `SessionConfig.credential_refs` (one OpenShell provider per ref, created at session start). Per-call refs go on `ExecRequest.cred_refs`. The `connection_id` set a backend may resolve is validated at the call site against the caller's allowlist (ACP profile `connection_ids`; INV capability grants).

### 2.9 ValidationResult (structured Degraded)

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationResult {
    Ok,
    Degraded { gaps: Vec<CapabilityGap> }, // was: warnings: Vec<String>
    Unsupported { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityGap {
    pub field: String,         // "network.egress_allow", "resource_limits.max_cpu_ms"
    pub reason: String,
    pub severity: GapSeverity, // Advisory | Significant
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GapSeverity { Advisory, Significant }
```

### 2.10 SandboxError (1 new variant)

Existing variants kept; add `Unsupported(String)` (returned by defaulted `spawn`/`update_policy`). `Timeout(Duration)` already exists and now covers `create_timeout`/`ready_timeout`.

### 2.11 SandboxContext (unchanged shape)

`{ id, backend_name, isolation_level, inner: Box<dyn Any + Send + Sync> }`. `inner` is `pub(crate)` but `downcast_ref` is `pub` (verified types.rs:178), so external backend crates recover state via `ctx.downcast_ref::<T>()` — the OpenShell cross-crate concern (Gap-10) is **not a gap**.

### 2.12 SandboxSessionGuard (NEW — concrete RAII type)

```rust
pub struct SandboxSessionGuard {
    backend: Arc<dyn SandboxBackend>,
    ctx: Option<SandboxContext>,
}
impl SandboxSessionGuard {
    pub fn new(backend: Arc<dyn SandboxBackend>, ctx: SandboxContext) -> Self { ... }
    pub fn ctx(&self) -> &SandboxContext { self.ctx.as_ref().expect("live") }
    /// Take ownership for an explicit async destroy with error propagation.
    pub fn take(mut self) -> (Arc<dyn SandboxBackend>, SandboxContext) { ... }
}
impl Drop for SandboxSessionGuard {
    fn drop(&mut self) {
        if let Some(ctx) = self.ctx.take() {
            let backend = self.backend.clone();
            tokio::spawn(async move { let _ = backend.destroy_session(ctx).await; });
        }
    }
}
```
This is the concrete enforcement behind tenet 5 — closes the tokio-cancellation leak for OpenShell/Firecracker that prose alone left open.

---

## 3. Lifecycle

```
SELECT  registry.select_for_request(req)  ->  SandboxSelection { backend, validation }
        (or select_by_name / select_for_criteria / select_for_manifest)
        platform pre-filter -> hard criteria -> is_available -> validate_config -> validate
CREATE  guard = SandboxSessionGuard::new(backend,
            backend.create_session(SessionConfig{policy,resource_limits,credential_refs,
                                                  create_timeout,ready_timeout,lifetime}))
  one-shot (EXC, INV):     loop { backend.execute(guard.ctx(), ExecRequest) }
  long-running (ACP, os):  let mut h = backend.spawn(guard.ctx(), SpawnRequest)?;
                           // h.stdin/h.stdout -> StdioTransport (JSON-RPC, many turns)
                           // h.close_stdin(); grace; h.signal(Term); h.signal(Kill); h.wait()
  supervisor liveness:     if !backend.is_alive(guard.ctx()).await? { restart }
  optional (OpenShell):    backend.update_policy(guard.ctx(), PolicyDelta)  // network/models
DESTROY guard drops -> destroy_session guaranteed (idempotent, best-effort, aggregated errors)
```

- **Session reuse** is governed by `SandboxLifetime`. `PerCall` = create/destroy around one execute. `Scope` = held in `ExecutionContext` across a graph. `Supervisor` = held by `supervise_agent` across agent restarts (re-`spawn` into the surviving ctx).
- **bwrap caveat (load-bearing):** `supports_persistent_session = false` for Process/Bubblewrap is correct, but the cost lives in `execute` (bwrap fork + namespace per call), not `create_session` (cheap TempDir). Pairing a slow consumer with bwrap's session model does NOT amortize the per-call fork. Heavy reuse only benefits `supports_persistent_session = true` backends.

---

## 4. Capability negotiation (cheapest-first)

1. **Platform pre-filter (free):** drop backends whose `supported_platforms` excludes the current OS before any probe (kills "select bwrap on macOS").
2. **Hard criteria:** drop backends failing a `SelectionCriteria` hard requirement (`require_persistent_session`, `require_streaming`, `require_credential_bundle`, `require_kvm_isolation`, `require_hot_reload`). Skipped, NOT Degraded — avoids create-per-exec on Firecracker/OpenShell.
3. **`is_available` (cached):** memoized resource/reachability probe.
4. **`validate_config(&SessionConfig)`:** session-level no-silent-fallback (mandatory `extra`, credential reachability, resource_limits applicability, microVM path mapping). Runs when the caller has a config.
5. **`validate(&ExecRequest)`:** per-call no-silent-fallback; `Degraded { gaps }` names each unenforceable field; drop `Unsupported`.
6. **Sort:** `Ok` before `Degraded`, then lowest isolation ≥ `min`, then registration order.

EXC and INV hard-reject any `Degraded` (current strict policy preserved). The structured `severity` lets a future relaxed caller accept `Advisory`-only gaps without weakening EXC.

---

## 5. Registry + selection

```rust
impl SandboxRegistry {
    pub fn register(&mut self, b: Arc<dyn SandboxBackend>);
    pub fn list(&self) -> Vec<SandboxCapabilities>;
    pub fn select(&self, min: IsolationLevel) -> Result<Arc<dyn SandboxBackend>, SandboxError>;
    pub fn select_by_name(&self, name: &str) -> Result<Arc<dyn SandboxBackend>, SandboxError>;       // NEW
    pub fn select_for_request(&self, req: &ExecRequest) -> Result<SandboxSelection, SandboxError>;
    pub fn select_for_criteria(&self, c: &SelectionCriteria) -> Result<SandboxSelection, SandboxError>; // NEW
    pub fn select_for_manifest(&self, m: &SecurityManifest) -> Result<SandboxSelection, SandboxError>;  // now validate-aware
    pub fn default_backend(&self) -> Option<Arc<dyn SandboxBackend>>;
}

#[derive(Debug, Clone, Default)]
pub struct SelectionCriteria {
    pub min_isolation: IsolationLevel,
    pub name: Option<String>,
    pub config: Option<SessionConfig>,        // enables validate_config during selection
    pub request: Option<ExecRequest>,         // enables validate during selection
    pub require_persistent_session: bool,
    pub require_credential_bundle: bool,
    pub require_streaming: bool,
    pub require_hot_reload: bool,
    pub require_kvm_isolation: bool,
}
```

`select_for_manifest` now BUILDS an `ExecRequest` (and minimal `SessionConfig`) from manifest fields and runs the full validate-aware path — closing the bypass at registry.rs:235 (`self.select(manifest.min_isolation)`) where Bubblewrap's read-path Degraded is currently lost. `select_by_name` bypasses isolation tie-breaking but still runs platform / `is_available` / `validate_config` / `validate` and reports why a named backend was rejected.

---

## 6. Per-backend usage

- **Process (PolicyOnly, all platforms, non-persistent, kvm=false):** `create_session(cfg)` no-op unit inner, reads `cfg.policy.credentials`. `execute` = `tokio::process::Command` with `env_clear` + passthrough/overrides, timeout, `max_output_bytes`. `validate` → `Degraded` naming every restriction it can't honor. `spawn`/`update_policy`/`is_alive` defaults.
- **Bubblewrap (Container, Linux-only, non-persistent, kvm=false):** `is_available` MEMOIZED (`OnceLock<bool>` over `bwrap --version` — fixes the per-query fork at sandbox_linux.rs:215). `create_session` allocates TempDir into inner; reads `cfg.policy`. `execute` forks fresh bwrap. `validate` → `Degraded { gaps: [read_paths(per-path), resource_limits.*(no cgroups)] }`. Implements `spawn` for ACP (agent as long-lived bwrap entry process with piped stdio) — currently absent, a migration item. Move the old `SandboxPolicy` env-filter field to `cfg.policy.credentials`. `destroy_session` = no-op (TempDir Drop).
- **OpenShell (Container, all platforms, persistent, hot-reload, credential-bundle, kvm=false):** optional crate behind feature flag, tonic gRPC. `is_available` cached gateway probe. `validate_config` checks `credential_refs` reachability + `inference.backend`. `create_session` = gRPC `CreateSandbox` (honors `create_timeout`), YAML from policy, one provider per `cfg.credential_refs`. `execute` = `ExecSandbox`. `spawn` = `ExecSandbox` attached stdio → `SessionHandle`. `update_policy` = `policy update` (network/models). `is_alive` = gRPC Ping. `intercept_enabled` default false; when true, points the provider at `inference.intercept_endpoint` (caller-supplied). `destroy_session` = `DeleteSandbox`.
- **Firecracker / Cloud-Hypervisor (Hypervisor, Linux-KVM, persistent, kvm=true):** `required_extra_keys()` = `["kernel_image","rootfs"]`. `is_available` checks `/dev/kvm`. `validate_config` rejects missing kernel/rootfs and missing `host_to_guest_paths` when read/write paths are set. `create_session` boots microVM (honors `create_timeout` + `ready_timeout`), guest agent PID 1, vsock. `execute` = vsock command dispatch. `spawn` keeps vsock open → `SessionHandle` whose `ChildControl::signal` sends a vsock message and `shutdown` issues the Firecracker poweroff API. `update_policy` default (network is host iptables on the tap, set at create). Cloud-Hypervisor adds GPU passthrough via `extra`.
- **gVisor (Hypervisor, Linux, persistent, syscall-filtering, kvm=false):** `runsc create/start`, cgroups from `resource_limits`. `execute` = `runsc exec`. `spawn` = `runsc exec -i`. `validate` → `Ok` for syscall-filtering + resource limits.
- **Docker / Podman (Container, all platforms, persistent, resource-limits, kvm=false):** `create_session` runs detached container. `execute` = `docker exec --no-tty`. `spawn` = `docker exec -i` → `SessionHandle`. `is_alive` = `docker inspect`. `destroy_session` = stop+rm.

---

## 7. Per-consumer usage

- **EXC (one-shot, strict):** `select_for_request(req)` with `min_isolation=OsLevel`; hard-error on empty registry; hard-reject `Degraded`. Fix: create once per **graph** held in `ExecutionContext` (new `sandbox_session: Option<SandboxSessionGuard>` alongside `sandbox_registry`), reuse `execute` across nodes (replaces per-node create/destroy at exc.rs:101-119). Migration: `create_session(SessionConfig::default())`.
- **INV / CapabilitySystem (one-shot, per-turn, latency-sensitive):** thread a session through `invoke_with_timeout` — add `ctx: Option<&SandboxContext>` (or store `sandbox_session` on the caller's `ExecutionContext` and pass it in). When a live ctx exists, skip `create_session` and DO NOT `destroy_session` per turn (deferred to the graph-end guard) — this is what removes the create-per-call tax at capability/mod.rs:315-330. Remove `BashCapability`'s direct `tokio::process::Command` spawn when a registry is present. Update the Degraded handling at capability/mod.rs:295 (`Degraded { warnings }` → `Degraded { gaps }`).
- **ACP coding-agent spawn (long-running, stdio):** add an optional `sandbox: Option<SandboxProfile>` (`{ min_isolation, name: Option<String>, connection_ids }`) to `AcpAgentProfile`; `None` keeps today's direct-spawn path. When set: build `SpawnRequest` from the profile + `cred_refs`; `select_for_criteria { min_isolation: Container, require_streaming: true, config }`; create guarded session; `spawn` → `SessionHandle`. Widen `StdioTransport` fields to the trait-object types and add `StdioTransport::from_parts(stdin, stdout)`; `read_response` works unchanged (single `&mut self`, no concurrent read/write — no split needed). Refactor `AcpSession` to hold `SessionHandle` instead of `(Child, StdioTransport)`; `close()` uses `close_stdin` → grace → `signal(Term)` → `signal(Kill)`; `Drop` delegates to the handle. **TerminalManager must be sandboxed:** thread `guard.ctx()` + the backend into `CapabilityReverseHandler`/`TerminalManager` so `terminal/create` reverse calls route through `backend.execute`/`spawn` in the SAME sandbox, not raw `tokio::process::Command`.
- **apxm-os agents (long-running, supervised, ZERO apxm cargo deps):** in os-core, add `sandbox_name: Option<String>` and `sandbox_lifetime: SandboxLifetime`-equivalent string to `AgentManifest` (own crate, no apxm dep). In apxm-server, add `sandbox: Option<SandboxHint>` (`{ min_isolation: String, name: Option<String>, lifetime: String }`, all string-typed) to `SkillExecuteRequest`; the server deserializes and calls `select_for_criteria`. supervise_agent: for `Supervisor` lifetime hold the guard + `Option<SessionHandle>` across restarts and re-`spawn` into the surviving ctx (requires `agent_loop(..., handle: Option<SessionHandle>)`); for `Scope`/`Task` create+destroy inside the loop. Wire `Isolation::Subprocess` (currently dead code) to `backend.spawn`. Use `is_alive` before restart; map `ExitStatus.sandbox_exit` (OomKilled → OnFailure) into the RestartController.

---

## 8. Migration summary

- **Source-compatible:** `execute`/`destroy_session` signatures, `ExecResult`, `SandboxContext`, `IsolationLevel`, registry tie-break, EXC/INV strict Degraded rejection. `Debug` is NOT added as a supertrait.
- **One-line call-site change:** `create_session()` → `create_session(SessionConfig::default())` at EXC (exc.rs:101), INV (capability/mod.rs:315), DefaultBackend (backend.rs:146), and every test double (registry.rs:448, exc.rs:250, capability/mod.rs:773).
- **Additive default-inert:** `ExecRequest.{cred_refs,network_policy}`; the new `SandboxCapabilities` fields; `SandboxError::Unsupported`.
- **New defaulted methods:** `validate_config`, `required_extra_keys`, `is_alive`, `spawn`, `update_policy`, `supports_streaming`.
- **Breaking renames (mechanical):** `ValidationResult::Degraded { warnings: Vec<String> }` → `{ gaps: Vec<CapabilityGap> }` (compile error at sandbox_linux.rs:78-87, capability/mod.rs:295, registry.rs:443, backend.rs:239 until updated); `policy.rs::SandboxPolicy` → `SessionPolicy`.
- **Behavioral fixes:** EXC per-graph reuse; INV session threading (no create-per-call); `select_for_manifest` validates; `BashCapability` dual-spawn removed when registry present; `is_available` memoized; concrete `SandboxSessionGuard`.

## 9. Concurrency & teardown contract (normative)

- `update_policy` is safe to call concurrently with `execute` on the same ctx. Backends that cannot must serialize internally; the trait adds no lock.
- `destroy_session` MUST be idempotent and best-effort: attempt every cleanup step independently, aggregate failures. `SandboxSessionGuard` guarantees the CALL; the backend owns whether work completes.
- `SpawnRequest.env` zeroization is tied to `SessionHandle` drop (child may outlive the request), not "after child exits". `ExecRequest.env` zeroization is tied to one-shot child exit.

---

## Migration plan (ordered)

1. Step 1 (runtime types, mechanical rename): in crates/runtime/engine/src/sandbox/backend.rs change ValidationResult::Degraded { warnings: Vec<String> } to { gaps: Vec<CapabilityGap> }; add CapabilityGap + GapSeverity. Add SandboxError::Unsupported. Fix the only in-crate constructors: registry.rs:443 test double and backend.rs:239 test. This is the first commit so everything downstream compiles against the new shape.
2. Step 2 (runtime types, additive): in types.rs add Platform enum; extend SandboxCapabilities with supports_persistent_session/hot_reload_policy/credential_bundle/kvm_isolation + supported_platforms; add cred_refs + network_policy to ExecRequest (Default inert). Add SpawnRequest, SessionHandle, ChildControl, Signal, ExitStatus, SandboxExitReason, SessionConfig, SandboxLifetime, ResourceLimits. Add SessionPolicy + sub-structs (FsPolicy/ProcPolicy/NetworkPolicy/InferencePolicy/CredPolicy/PolicyDelta/SyscallProfile) replacing policy.rs::SandboxPolicy.
3. Step 3 (trait): change create_session to take SessionConfig; add defaulted validate_config, required_extra_keys, is_alive, spawn, update_policy, supports_streaming. Update DefaultBackend (backend.rs:146) to accept and ignore the arg. Do NOT add the Debug supertrait. Compile the crate (lib only) to confirm defaults cover all existing impls.
4. Step 4 (registry): add SandboxSelection-returning select_by_name, select_for_criteria(SelectionCriteria), and rewrite select_for_request + select_for_manifest into the unified pipeline: platform pre-filter -> hard criteria -> is_available -> validate_config (if config present) -> validate -> sort. select_for_manifest now builds ExecRequest + minimal SessionConfig. Update all registry tests (create_session arg, Degraded gaps, DefaultBackend usages).
5. Step 5 (RAII guard): add SandboxSessionGuard with Drop spawning destroy_session. Place it in the sandbox module and export it.
6. Step 6 (Process + Bubblewrap backends): migrate sandbox_linux.rs and process.rs to read SessionConfig.policy; memoize Bubblewrap::is_available with OnceLock<bool>; convert validate to emit Degraded { gaps } with named fields + GapSeverity; move bwrap's old SandboxPolicy env-filter field to cfg.policy.credentials; add supported_platforms (bwrap=Linux, Process=all). Implement bwrap spawn() for ACP (agent as long-lived bwrap entry, piped stdio). Run the sandbox unit + integration tests.
7. Step 7 (EXC consumer): add sandbox_session: Option<SandboxSessionGuard> to ExecutionContext; in exc.rs create the guarded session once at graph start (SessionConfig{lifetime:Scope,..default}) and reuse execute across nodes; remove per-node create/destroy at exc.rs:101-119; update the Degraded match. Run EXC handler tests including the empty-registry rejection.
8. Step 8 (INV consumer): thread the session into CapabilitySystem::invoke_with_timeout (ctx: Option<&SandboxContext> param, or read ExecutionContext.sandbox_session); skip create_session and defer destroy when a live ctx exists (removes create-per-call at capability/mod.rs:315-330); update the Degraded handling at capability/mod.rs:295; remove BashCapability's direct tokio::process::Command spawn when a registry is configured. Verify INV bash/read/write routing tests + latency on bwrap (<50 ms warm per turn).
9. Step 9 (ACP protocol): widen StdioTransport fields to Box<dyn AsyncWrite/AsyncRead + Send + Unpin>; add StdioTransport::from_parts; confirm read_response compiles unchanged (single &mut self). Add SandboxProfile { min_isolation, name, connection_ids } as optional field on AcpAgentProfile (None preserves direct spawn).
10. Step 10 (ACP session): refactor AcpSession to hold SessionHandle instead of (Child, StdioTransport); when profile.sandbox is set, build SpawnRequest (+cred_refs validated against profile.connection_ids), select_for_criteria{require_streaming,config}, create guarded session, spawn, wrap stdio in StdioTransport::from_parts; rewrite close() to close_stdin -> grace -> signal(Term) -> signal(Kill) -> wait and Drop to delegate to the handle. Thread guard.ctx() + backend into CapabilityReverseHandler/TerminalManager so terminal/create reverse calls route through backend.execute/spawn in the same sandbox. e2e-test a sandboxed codex/claude ACP session including a reverse terminal call.
11. Step 11 (optional heavy backends, feature-gated, parallelizable): implement Docker/Podman, gVisor, OpenShell (tonic gRPC, validate_config credential+inference reachability, update_policy, is_alive Ping, one provider per credential_ref), Firecracker (required_extra_keys kernel/rootfs, create_timeout+ready_timeout, vsock ChildControl signal/shutdown, host_to_guest_paths validation). Each lands behind its own crate/feature; none block steps 1-10.
12. Step 12 (apxm-os wiring, no apxm cargo dep): add sandbox_name: Option<String> + sandbox_lifetime string to AgentManifest in os-core; add sandbox: Option<SandboxHint> (string-typed) to apxm-server SkillExecuteRequest; server deserializes and calls select_for_criteria. In supervise_agent map AgentManifest.sandbox/isolation to a SandboxHint, hold the guard + Option<SessionHandle> across restarts for Supervisor lifetime (agent_loop(..., handle) signature change), wire Isolation::Subprocess to spawn, call is_alive before restart, map ExitStatus.sandbox_exit to RestartPolicy. e2e-test a long-lived Docker-backed os agent through a restart.
13. Step 13 (cleanup): delete policy.rs::SandboxPolicy once no references remain; run the full workspace test suite + clippy; confirm no vendor type leaks into core structs (grep for backend-specific identifiers in types.rs/backend.rs/registry.rs).

## Changes the e2e verification forced on the draft

- Dropped the `+ std::fmt::Debug` supertrait addition. Verified registry/selection debug output (registry.rs:252-278) uses `capabilities().name`, not `{:?}` on the backend, and the test double DegradedBackend (registry.rs:433) does not impl Debug. Adding the supertrait was a gratuitous source break (OpenShell RC-4, Bubblewrap Gap-5).
- Added `validate_config(&SessionConfig) -> ValidationResult` (defaulted Ok) to the trait. The draft's `validate(&ExecRequest)` structurally cannot see SessionConfig fields (resource_limits, credential refs, policy.extra), so session-level no-silent-fallback was unenforceable. Flagged by OpenShell (RC-3), Firecracker (gap 1-2), per-turn (gap 4), and the registry now runs it during selection.
- Added `required_extra_keys() -> &'static [&'static str]` (defaulted empty). Resolves Firecracker's 'SessionConfig::default() hides mandatory kernel/rootfs' invisible-invariant problem at the type/registry level instead of at boot time.
- Added `is_alive(&ctx) -> Result<bool>` (defaulted Ok(true)). The draft had no liveness probe; the apxm-os supervisor (os-longrunning verdict gaps 1,8) cannot distinguish 'agent exited' from 'sandbox crashed' without it.
- Replaced `SessionConfig.credential_bundle_id: Option<String>` with `credential_refs: Vec<CredentialRef>` (OpenShell RC-5). Heavy backends create one provider per credential at session start; a single bundle id forced a redundant secondary lookup duplicating CredentialRef semantics.
- Added `create_timeout` and `ready_timeout` to SessionConfig (OpenShell RC-2, Firecracker required-change 3). A seconds-scale create with no caller-supplied bound can hang a graph; ready_timeout separates VM-boot readiness from per-command execution time so a timeout error is attributable.
- Added `SandboxLifetime { PerCall, Scope, Supervisor }` to SessionConfig and made session ownership explicit per consumer. The draft asserted 'create once per graph/agent' as prose; the os-longrunning and per-turn verdicts showed the interface never said WHERE the ctx is held across restarts/turns. Scope→ExecutionContext, Supervisor→supervise_agent holding the guard + re-spawn into surviving ctx.
- Added a concrete `SandboxSessionGuard` RAII type with `impl Drop` (per-turn RC, os-longrunning). The draft promised 'RAII-guaranteed even on panic' as documentation only; without a real type the tokio-cancellation leak between execute and destroy stays open for OpenShell/Firecracker.
- Widened SessionHandle stdio to trait objects AND added `close_stdin`, `shutdown`, `id` to ChildControl. ACP verdict: StdioTransport is typed to concrete ChildStdin/ChildStdout (protocol.rs:58-59,64) — the draft's claim that handle.stdin/stdout drop into 'the existing StdioTransport' does not compile; close_stdin preserves the drop-stdin-before-signal grace sequence (session.rs:302-333); shutdown separates guest-process-signal from VM-poweroff (Firecracker); id() preserves the Unix child.id() SIGTERM path.
- Retracted the ACP 'needs split() handles' requirement. Verified protocol.rs:157 read_response interleaves read_message + send_response on a single &mut self and never reads/writes concurrently, so a single struct owning both trait-object halves is sufficient; only `from_parts` plus the field-type widening is needed.
- Enriched ExitStatus with `sandbox_exit: Option<SandboxExitReason>` (os-longrunning gap 8). Lets the supervisor map OomKilled/SandboxDeleted to the correct RestartPolicy instead of a generic TaskExit.
- Added `host_to_guest_paths` to SessionPolicy and the rule that a microVM backend returns Degraded naming read/write paths when no mapping is supplied (Firecracker gap 3 / required-change 4). Container backends leave it empty (implicit same-path bind).
- Added `supports_kvm_isolation` capability + `require_kvm_isolation` criterion (Firecracker gap 8). gVisor and Firecracker both map to IsolationLevel::Hypervisor; without a sub-flag a caller needing true KVM could silently get gVisor's user-space kernel. Kept the enum ordinal unsplit since it is used for ordering.
- Promoted inference intercept endpoint to a typed `InferencePolicy.intercept_endpoint: Option<String>` supplied by the caller (OpenShell gap 6 / RC-7), instead of leaking apxm-server's address through the opaque `extra` blob.
- Made the bwrap latency characterization normative in §3: supports_persistent_session=false is correct, but the fork cost is in execute() not create_session(), so pairing a slow consumer with bwrap's session model does not amortize anything (per-turn gap 6, required-change 4).
- Specified the SpawnRequest.env zeroization contract as tied to SessionHandle drop (not 'after child exits'), distinct from ExecRequest.env tied to one-shot exit, and wrapped it in Zeroizing (ACP gap 7 / required-change 6).
- Added the normative §9 concurrency/teardown contract: update_policy safe-concurrent-with-execute, destroy_session idempotent/best-effort/aggregated-errors (OpenShell RC-6, Firecracker required-change 7).
- Confirmed Gap-10 (cross-crate inner access) is NOT a gap: ctx.downcast_ref is pub (types.rs:178). Documented in §2.11 so a future implementer does not re-litigate it.
- Added the apxm-os wire path concretely: os-core gains sandbox_name + lifetime string fields (own crate, no apxm dep), apxm-server's SkillExecuteRequest gains a string-typed SandboxHint, server deserializes and calls select_for_criteria (os-longrunning RC-3,4,5 — the draft sketched 'wire-level mapping' but the envelope had no field, confirmed os-client/src/lib.rs:34 mirrors args+session_id only).

## Open questions

- Should INV thread the session via an explicit `ctx: Option<&SandboxContext>` parameter on invoke_with_timeout (touches every call site, e.g. inv_tool.rs:199) or via a field read from ExecutionContext (fewer signature changes but couples CapabilitySystem to the executor context shape)? The spec allows either; the call-site count favors the field approach if ExecutionContext is already in scope at every invoke.
- For SandboxLifetime::Supervisor, agent_loop must accept a pre-existing SessionHandle, but a SessionHandle owns non-Clone trait-object streams. Does the supervisor pass &mut SessionHandle (borrow across the loop iteration) or move-and-return it each restart? Move-and-return is cleaner for ownership but forces agent_loop to give the handle back even on panic — needs a guard wrapper of its own.
- The microVM host_to_guest_paths translation: should the runtime dispatch layer rewrite ExecRequest.working_dir/read_paths/write_paths into guest paths before calling execute, or should the backend do the rewrite internally from SessionPolicy.host_to_guest_paths? Backend-internal keeps the consumer microVM-agnostic but means every microVM backend reimplements the mapping.
- Credential allowlist enforcement point: the spec validates connection_id against the caller's allowlist 'at the call site', but for proxy injection the backend resolves at egress. Should the registry/dispatch layer reject disallowed connection_ids before create_session, or is the OPA proxy's fail-closed swap sufficient? Belt-and-suspenders (both) is safest but the spec should name the authoritative gate.
- Does apxm-server already have a place to register heavy backends (Docker/OpenShell/Firecracker) at startup, or does the SandboxHint path from apxm-os assume backends that may not be registered? If select_for_criteria fails for a requested sandbox_name, the supervisor needs a defined fallback (fail the agent vs degrade to a weaker backend) — not specified.
- OpenShell inference intercept_endpoint is caller-supplied, but for the apxm-os wire path the caller is apxm-server itself routing its own OpenAI-compat endpoint back into a sandbox it created. Is there a loop/self-reference hazard (sandboxed agent calls back into the same server that supervises it) that needs a distinct internal endpoint or auth scope?
