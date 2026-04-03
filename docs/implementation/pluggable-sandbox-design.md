# Pluggable Sandbox Architecture for APXM

> **Status**: Implemented (Phase 1)
> **Date**: 2026-03-23
> **Crate**: `apxm-sandbox`
> **Principle**: APXM defines the sandbox *interface*, not the *implementation*.

## Core Principle

APXM is a platform-agnostic Program Execution Model (compiler + runtime). It never implements OS-level sandboxing directly — no `Command::new`, no bubblewrap, no seccomp, no Seatbelt, no Docker. Instead, it defines a `SandboxBackend` trait that host applications implement using their own isolation technology.

This is the same relationship as:
- LLVM IR → target backends (x86, ARM, RISC-V are external)
- Kubernetes CRI → container runtimes (containerd, CRI-O are external)
- APXM `LLMBackend` → provider implementations (OpenAI, Anthropic are external)

## LLM Calls Do NOT Need Sandboxing

A critical architectural insight: LLM operations (ASK, THINK, REASON, PLAN, REFLECT, etc.) are HTTP API calls. They don't spawn processes, don't touch the filesystem, don't open sockets beyond the API connection. **The sandbox is only for tool execution** — INV ops, capability calls, and code execution.

```
┌─────────────────────┐     ┌──────────────────────┐
│   LLMBackend        │     │   SandboxBackend      │
│   (who answers)     │     │   (who executes)      │
├─────────────────────┤     ├──────────────────────┤
│ ASK, THINK, REASON  │     │ INV, tool calls,     │
│ PLAN, REFLECT, etc. │     │ code execution       │
├─────────────────────┤     ├──────────────────────┤
│ HTTP request        │     │ Process spawn         │
│ No sandbox needed   │     │ NEEDS sandbox         │
└─────────────────────┘     └──────────────────────┘
     Orthogonal concerns — both pluggable, both injected by host
```

## Architecture

```
┌────────────────────────────────────────────────────────────┐
│                    APXM (platform-agnostic)                │
│                                                            │
│  ┌──────────────┐    ┌──────────────────────────────────┐  │
│  │ AIS Graph    │───▸│ SecurityManifest                 │  │
│  │ (compiled)   │    │ (compiler: classify_op → tiers)  │  │
│  └──────────────┘    └──────────┬───────────────────────┘  │
│                                 │                          │
│                                 ▼                          │
│  ┌──────────────────────────────────────────────────────┐  │
│  │   SandboxBackend trait  (apxm-sandbox crate)        │  │
│  │                                                      │  │
│  │   fn capabilities() -> SandboxCapabilities           │  │
│  │   fn is_available() -> bool                          │  │
│  │   fn validate(&ExecRequest) -> ValidationResult      │  │
│  │   fn create_session() -> SandboxContext              │  │
│  │   fn execute(&ctx, ExecRequest) -> ExecResult        │  │
│  │   fn destroy_session(ctx)                            │  │
│  └──────────────────────────────────────────────────────┘  │
│                                                            │
│  ┌──────────────────────────────────────────────────────┐  │
│  │   SandboxRegistry (holds Arc<dyn SandboxBackend>)   │  │
│  │   select(min_isolation) -> best available backend    │  │
│  └──────────────────────────────────────────────────────┘  │
│                                                            │
│  ┌──────────────────────────────────────────────────────┐  │
│  │   DefaultBackend (closure-based, zero platform deps)│  │
│  └──────────────────────────────────────────────────────┘  │
│                                                            │
└────────────────────────────────────────────────────────────┘
                           │
                           │ Host applications IMPLEMENT the trait
                           │
    ┌──────────────────────┼──────────────────────┐
    ▼                      ▼                      ▼
┌──────────────┐  ┌──────────────────┐  ┌──────────────────┐
│ Codex        │  │ Gemini CLI       │  │ Custom Host      │
│ bwrap+seccomp│  │ Seatbelt/bwrap   │  │ Docker/Wasm/VM   │
│ (in codex-rs)│  │ (in gemini-cli)  │  │ (in their crate) │
└──────────────┘  └──────────────────┘  └──────────────────┘
```

**Key difference from v1 design**: APXM ships ZERO backend implementations (except `DefaultBackend`, which is a closure wrapper with no platform code). All real backends live in host applications.

## Crate: `apxm-sandbox`

**Location**: `crates/apxm-sandbox/`
**Dependencies**: Only `async-trait`, `futures`, `serde`, `serde_json`, `thiserror`, `tracing` — no platform deps.

### Files

| File | Purpose |
|------|---------|
| `backend.rs` | `SandboxBackend` trait + `DefaultBackend` + `ValidationResult` |
| `types.rs` | `ExecRequest`, `ExecResult`, `SandboxContext`, `SandboxCapabilities`, `IsolationLevel` |
| `error.rs` | `SandboxError` enum |
| `manifest.rs` | `SecurityManifest`, `NodeSandboxReq`, `classify_op()`, tier constants |
| `registry.rs` | `SandboxRegistry` — holds backends, selects best match |

### Key Types

```rust
// IsolationLevel — ordered enum for backend selection
enum IsolationLevel { None, PolicyOnly, OsLevel, Container, Hypervisor, Wasm, Remote }

// ExecRequest — what flows from APXM to the sandbox
struct ExecRequest {
    program: String, args: Vec<String>, working_dir: Option<PathBuf>,
    env: HashMap<String, String>, stdin_data: Option<String>,
    timeout: Duration, max_output_bytes: usize,
    read_paths: Vec<PathBuf>, write_paths: Vec<PathBuf>,
    needs_network: bool, needs_process_spawn: bool,
    origin_op: Option<String>, origin_node_id: Option<u64>,
    min_isolation: IsolationLevel,
}

// ExecResult — what comes back
struct ExecResult {
    success: bool, exit_code: Option<i32>,
    stdout: String, stderr: String,
    duration: Duration, timed_out: bool,
}
```

### The Trait

```rust
#[async_trait]
pub trait SandboxBackend: Send + Sync {
    fn capabilities(&self) -> SandboxCapabilities;
    fn is_available(&self) -> bool;
    fn validate(&self, request: &ExecRequest) -> ValidationResult;
    async fn create_session(&self) -> Result<SandboxContext, SandboxError>;
    async fn execute(&self, ctx: &SandboxContext, request: ExecRequest) -> Result<ExecResult, SandboxError>;
    async fn destroy_session(&self, ctx: SandboxContext) -> Result<(), SandboxError>;
}
```

**Contract**:
- LLM calls (ASK, THINK, REASON) NEVER go through the sandbox
- Only tool execution (INV, capabilities) uses the sandbox
- `create_session()` is called once per graph execution
- `execute()` is called per INV/tool node
- `destroy_session()` is always called (even on failure)

## Runtime Integration

The `SandboxRegistry` is injected into:
- `Runtime` struct (via `set_sandbox_registry()`)
- `ExecutionContext` (propagated to all handlers via `sandbox_registry` field)

```rust
// Runtime holds the registry
pub struct Runtime {
    sandbox_registry: Arc<SandboxRegistry>,
    // ... other fields
}

// ExecutionContext carries it to handlers
pub struct ExecutionContext {
    pub sandbox_registry: Arc<SandboxRegistry>,
    // ... other fields
}
```

Follows the exact same pattern as `LLMRegistry`:
1. Host creates registry + registers backends
2. Host calls `runtime.set_sandbox_registry(registry)`
3. Runtime passes it to every `ExecutionContext`
4. INV handler queries the registry to get a backend

## AIS Operation Tiers

| Tier | Operations | Min Isolation | Rationale |
|------|-----------|---------------|-----------|
| **T0: Pure** | THINK, REASON, PLAN, REFLECT, VERIFY, EXPLAIN, SUMMARIZE, CRITIQUE, DECIDE, SCORE, RANK, CLASSIFY, EXTRACT, TRANSFORM, SELECT, MERGE, SPLIT, WAIT_ALL, FENCE, BRANCH, SWITCH | `None` | LLM-only, no side effects |
| **T1: Memory** | QMEM, UMEM, SMEM, AMEM, RMEM, STM_PUT, STM_GET | `PolicyOnly` | Agent memory stores |
| **T2: I/O** | ASK (w/ tools), INV, UPDATE_GOAL, EMIT, COMMUNICATE | `OsLevel` | External I/O |
| **T3: Privileged** | GUARD, CLAIM, RELEASE, RESUME, DELEGATE, SPAWN | `Container` | Multi-agent coordination |

Unknown operations default to T2 (safe by default).

> **Note:** Many ops listed in the tier table (e.g., EXPLAIN, SUMMARIZE, SCORE,
> RANK, CLASSIFY, EXTRACT, TRANSFORM, SELECT, SPLIT, CRITIQUE, DECIDE, SMEM,
> AMEM, RMEM, STM_PUT, STM_GET, EMIT, RELEASE) are forward-compatibility
> entries in `classify_op()` — they do not exist as `AISOperationType` enum
> variants yet. They are pre-classified so that adding them later does not
> require updating the sandbox tier logic.

## Host Integration Examples

### Codex (bwrap + seccomp)

```rust
// codex-rs/core/src/apxm_adapter/sandbox_bridge.rs
struct CodexSandboxBridge {
    sandbox_manager: Arc<SandboxManager>,
    policy: SandboxPolicy,
    file_system_policy: FileSystemSandboxPolicy,
}

#[async_trait]
impl SandboxBackend for CodexSandboxBridge {
    fn capabilities(&self) -> SandboxCapabilities {
        SandboxCapabilities {
            isolation_level: IsolationLevel::Container,
            supports_filesystem_restriction: true,
            supports_network_restriction: true,
            supports_syscall_filtering: true,
            supports_resource_limits: false,
            name: "codex-bwrap".into(),
            version: env!("CARGO_PKG_VERSION").into(),
        }
    }

    async fn execute(&self, _ctx: &SandboxContext, req: ExecRequest) -> Result<ExecResult, SandboxError> {
        // Translate ExecRequest → Codex's CommandSpec
        // Delegate to SandboxManager::transform()
        // Run via Codex's execute_exec_request()
    }
    // ...
}
```

### Gemini CLI (Seatbelt/bwrap/Windows)

```typescript
// Gemini's SandboxManager already has the right shape:
// prepareCommand(req: SandboxRequest) -> SandboxedCommand
// A thin Rust wrapper would implement SandboxBackend
```

### Custom Docker Integration

```rust
// Some external crate: apxm-sandbox-docker
struct DockerSandboxBackend { image: String, client: Docker }

#[async_trait]
impl SandboxBackend for DockerSandboxBackend {
    // create_session() → docker create + start
    // execute() → docker exec
    // destroy_session() → docker rm -f
}
```

## Implementation Status

### Phase 1 (DONE)
- [x] `apxm-sandbox` crate with trait, types, error, manifest, registry
- [x] `DefaultBackend` (closure-based, zero platform deps)
- [x] `SandboxRegistry` with selection logic
- [x] `SecurityManifest` with `classify_op()` for all 39 AIS ops
- [x] Wired into `Runtime` and `ExecutionContext`
- [x] 29 tests passing (15 unit + 14 integration)

### Phase 2 (DONE)
- [x] `CodexSandboxBridge` in `codex-rs/core/src/apxm_adapter/sandbox_bridge.rs`
- [x] `GeminiSandboxBridge` in `google/gemini-cli/packages/apxm-bridge/` (19 tests)
- [x] Connect INV handler to sandbox registry (`CapabilitySystem.invoke_with_timeout()` routes through `SandboxBackend`)
- [x] `BashCapability` and `UserToolCapability` override `to_exec_request()`
- [ ] Compiler emits `SecurityManifest` during graph compilation
- [ ] `apxm sandbox status` CLI command

### Phase 3
- [ ] AgentMate unification: `am-sandbox` delegates to `apxm-sandbox` backends
- [ ] `apxm sandbox analyze graph.json` CLI command

### Phase 4
- [ ] External backend examples (Docker, Wasm, Remote)
- [ ] Full `apxm sandbox` CLI suite
- [ ] Configuration schema (`[sandbox]` in config.toml)

## Key Design Decisions

1. **APXM ships zero platform code** — no bwrap, no seccomp, no Docker client, no Seatbelt. All backends are external.
2. **Detection is the host's job** — APXM never calls `which("bwrap")`. The host knows its environment.
3. **Two orthogonal pluggable interfaces** — `LLMBackend` (who answers) and `SandboxBackend` (who executes). LLM calls bypass the sandbox entirely.
4. **One session per graph** — `create_session()` is called once, amortizing container/VM startup.
5. **Select lowest qualifying** — the registry picks the cheapest backend that meets the security manifest's minimum isolation level.
6. **Safe by default** — unknown AIS ops are classified as T2 (I/O), requiring at least OS-level isolation.
