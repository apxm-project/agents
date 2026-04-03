# APXM Backend Unification — Implementation Specification

> **Historical document** — this implementation plan has been completed. See [`backends-and-models.md`](backends-and-models.md) for the current architecture.
>
> **Status: COMPLETED (2026-04-03)** — All phases have been implemented and legacy removed.
> For the full configuration format, see [`docs/reference/config.md`](../reference/config.md).

---

## Overview

Unify the current fragmented LLM configuration system (`credentials.toml`, `models.toml`, `config.toml`) into a coherent **Backend → Model → Endpoint** hierarchy with proper CLI support.

## Current State (Problems)

```
~/.apxm/credentials.toml    → apxm llm add/list/remove/test
~/.apxm/models.toml         → apxm models list/health (read-only)
~/.apxm/config.toml         → [[llm_backends]] (manual edit only)
```

**Issues:**
1. `credentials.toml` stores "backends" but calls them "credentials"
2. `apxm llm` manages backends, not LLMs
3. `apxm models` is read-only, can't add models
4. No concept of backend `type` (cloud/onprem/local)
5. No lifecycle management for local backends (start/stop vLLM)
6. ModelRouter loads from `models.toml` separately from credentials

## Target State

```
~/.apxm/config.toml         → Single source of truth
  ├── [[backends]]          → apxm backend add/list/remove/test/start/stop
  │     ├── type: cloud|onprem|local
  │     ├── protocol: openai|anthropic|google|ollama|vllm
  │     ├── endpoint, api_key, headers
  │     └── [[backends.models]]  → Models hosted on this backend
  └── [routing]             → Default model, operation routes, fallbacks
```

## Hierarchy Definition

```
BACKEND
├── name: string (unique identifier)
├── type: "cloud" | "onprem" | "local"
├── protocol: ProviderProtocol (openai, anthropic, google, ollama, vllm)
├── endpoint: URL
├── api_key: string | "env:VAR_NAME"
├── headers: HashMap<String, String>
├── docker: Option<DockerConfig>  // For local backends
└── models: Vec<ModelConfig>
    └── MODEL
        ├── id: string (model identifier sent to API)
        ├── aliases: Vec<String>
        ├── context_window: usize
        ├── cost_per_1k_input: f64
        ├── cost_per_1k_output: f64
        ├── supports_vision: bool
        ├── supports_functions: bool
        ├── supports_thinking: bool
        └── tags: Vec<String>
```

## Implementation Plan

### Phase 1: Core Types (apxm-core)

**File: `crates/apxm-core/src/types/backend.rs`** (NEW)

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackendType {
    Cloud,
    OnPrem,
    Local,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendConfig {
    pub name: String,
    #[serde(rename = "type")]
    pub backend_type: BackendType,
    pub protocol: ProviderProtocol,
    pub endpoint: Option<String>,
    pub api_key: Option<String>,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(default)]
    pub models: Vec<ModelConfig>,
    pub docker: Option<DockerConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    pub id: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub context_window: usize,
    #[serde(default)]
    pub cost_per_1k_input: f64,
    #[serde(default)]
    pub cost_per_1k_output: f64,
    #[serde(default)]
    pub supports_vision: bool,
    #[serde(default)]
    pub supports_functions: bool,
    #[serde(default)]
    pub supports_thinking: bool,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockerConfig {
    pub image: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub command: Vec<String>,
    pub model_path: Option<String>,
    pub tensor_parallel: Option<usize>,
}
```

### Phase 2: Unified Config (apxm-driver)

**File: `crates/apxm-driver/src/config/mod.rs`**

Update `ApXmConfig` to include:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ApXmConfig {
    // ... existing fields ...
    
    /// Unified backend definitions (replaces llm_backends)
    #[serde(default)]
    pub backends: Vec<BackendConfig>,
    
    /// Legacy: still supported for migration
    #[serde(default)]
    pub llm_backends: Vec<LlmBackendConfig>,
}
```

### Phase 3: Backend Store (apxm-credentials → apxm-backends-store)

**Option A:** Extend `apxm-credentials` to handle full backend configs
**Option B:** Create new `BackendStore` that wraps both old and new formats

We'll use Option A (extend existing crate):

**File: `crates/apxm-credentials/src/backend.rs`** (NEW)

```rust
pub struct BackendStore {
    config_path: PathBuf,
}

impl BackendStore {
    pub fn open() -> Result<Self, BackendError>;
    pub fn add(&self, backend: BackendConfig) -> Result<(), BackendError>;
    pub fn remove(&self, name: &str) -> Result<(), BackendError>;
    pub fn get(&self, name: &str) -> Result<Option<BackendConfig>, BackendError>;
    pub fn list(&self) -> Result<Vec<BackendConfig>, BackendError>;
    pub fn update(&self, name: &str, backend: BackendConfig) -> Result<(), BackendError>;
    
    // Migration
    pub fn migrate_from_credentials(&self) -> Result<usize, BackendError>;
}
```

### Phase 4: CLI Commands (apxm-cli)

**File: `crates/apxm-cli/src/main.rs`**

Add new `Backend` command group:

```rust
#[derive(Subcommand, Debug)]
enum BackendAction {
    /// List all registered backends
    List,
    /// Add a new backend
    Add {
        name: String,
        #[arg(long)]
        type_: BackendType,
        #[arg(long)]
        protocol: ProviderProtocol,
        #[arg(long)]
        endpoint: Option<String>,
        #[arg(long)]
        api_key: Option<String>,
        #[arg(long, value_parser = parse_header)]
        header: Vec<(String, String)>,
    },
    /// Remove a backend
    Remove { name: String },
    /// Test backend connectivity
    Test { name: Option<String> },
    /// Show backend health (circuit breakers)
    Health,
    /// Start a local backend (if docker config exists)
    Start { name: String },
    /// Stop a local backend
    Stop { name: String },
    /// Show backend status (running/stopped)
    Status,
}
```

Keep `apxm llm` as deprecated alias pointing to `apxm backend`.

### Phase 5: Runtime Integration

**File: `crates/apxm-driver/src/runtime/llm.rs`**

Update `configure_llm_registry` to:
1. First try loading from `[[backends]]` in config.toml
2. Fall back to `credentials.toml` for migration
3. Register all backend models into ModelRegistry automatically

**File: `crates/apxm-runtime/src/model_router/registry.rs`**

Update `ModelRegistry` to accept models from BackendConfig directly (not just models.toml).

### Phase 6: Docker Lifecycle (for local backends)

**File: `crates/apxm-credentials/src/docker.rs`** (NEW)

```rust
pub struct DockerManager;

impl DockerManager {
    pub fn start(backend: &BackendConfig) -> Result<String, DockerError>;  // Returns container ID
    pub fn stop(container_id: &str) -> Result<(), DockerError>;
    pub fn status(backend_name: &str) -> Result<ContainerStatus, DockerError>;
    pub fn logs(container_id: &str, tail: usize) -> Result<String, DockerError>;
}
```

### Phase 7: dekk Integration

**File: `~/projects/dekk/.dekk.toml` changes**

Update APXM's .dekk.toml to use new commands:

```toml
[commands.backend]
description = "Manage inference backends"
group = "Configuration"
add = { run = "apxm backend add", description = "Add backend" }
list = { run = "apxm backend list", description = "List backends" }
remove = { run = "apxm backend remove", description = "Remove backend" }
test = { run = "apxm backend test", description = "Test connectivity" }
health = { run = "apxm backend health", description = "Show health" }
start = { run = "apxm backend start", description = "Start local backend" }
stop = { run = "apxm backend stop", description = "Stop local backend" }
status = { run = "apxm backend status", description = "Show status" }

# Keep llm as deprecated alias
[commands.llm]
description = "Manage LLM credentials (deprecated, use 'backend')"
group = "Configuration"
# ... existing subcommands, but print deprecation warning
```

## File Changes Summary

| Crate | File | Action |
|-------|------|--------|
| apxm-core | `src/types/backend.rs` | CREATE |
| apxm-core | `src/types/mod.rs` | MODIFY (add pub mod backend) |
| apxm-credentials | `src/backend.rs` | CREATE |
| apxm-credentials | `src/docker.rs` | CREATE |
| apxm-credentials | `src/lib.rs` | MODIFY (add modules) |
| apxm-driver | `src/config/mod.rs` | MODIFY (add backends field) |
| apxm-driver | `src/runtime/llm.rs` | MODIFY (load from backends) |
| apxm-runtime | `src/model_router/registry.rs` | MODIFY (accept BackendConfig) |
| apxm-cli | `src/main.rs` | MODIFY (add BackendAction) |
| apxm (root) | `.dekk.toml` | MODIFY (add backend commands) |

## Migration Path

1. Old `credentials.toml` continues to work (read during registry config)
2. `apxm backend migrate` converts credentials.toml → config.toml [[backends]]
3. `apxm llm` commands print deprecation warning, delegate to `apxm backend`
4. After migration, credentials.toml can be deleted

## Test Plan

1. Unit tests for BackendConfig serialization
2. Unit tests for BackendStore CRUD
3. Integration test: add backend → list → test → remove
4. Integration test: migrate from credentials.toml
5. Integration test: start/stop local vLLM backend
6. E2E test: execute graph with backend from new config

## Success Criteria

- [x] `apxm backend add/list/remove/test` works
- [x] `apxm backend start/stop` manages Docker containers
- [x] `apxm backend health` shows circuit breaker state
- [x] Existing `credentials.toml` users continue to work
- [x] ModelRouter loads models from [[backends.models]]
- [x] All 400+ tests pass
- [x] Zero breaking changes to graph execution
