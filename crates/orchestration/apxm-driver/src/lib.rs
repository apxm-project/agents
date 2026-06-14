//! APxM Driver - Configuration, linker, and coordination
//!
//! This crate consolidates configuration and coordination:
//!
//! - **`config`**: TOML-based configuration (`~/.apxm/config.toml`)
//! - **`linker`**: High-level linker coordinating compiler/runtime
//! - **`compiler`**: Compiler wrapper for AirModule parsing and lowering
//! - **`runtime`**: Runtime executor for DAG execution
//!
//! # Architecture
//!
//! ```text
//!                      ┌─────────────────┐
//!                      │   apxm-driver   │
//!                      └────────┬────────┘
//!                               │
//!        ┌──────────────────────┼──────────────────────┐
//!        ▼                      ▼                      ▼
//!   ┌──────────┐          ┌──────────┐          ┌──────────┐
//!   │  config  │          │  linker  │          │ runtime  │
//!   │  TOML    │          │  Coord.  │          │  Exec    │
//!   │ Parsing  │          │  Logic   │          │  DAGs    │
//!   └──────────┘          └──────────┘          └──────────┘
//! ```

pub mod compiler;
pub mod config;
pub mod context_assembler;
pub mod error;
pub mod hooks;
pub mod linker;
pub mod runtime;
pub mod session_output;
pub mod skill_resolver;

// --- Config ---
pub use config::{
    ApXmConfig, ChatConfig, ConfigError, ExecutionStreamConfig, GenerateStreamConfig, HookConfig,
    HookEvent, RunEventsConfig, ServerAuthConfig, ServerConfig, ServerExecutionsConfig,
    ServerInferenceConfig, ServerMcpConfig, ServerObservabilityConfig, ServerProcessConfig,
    ServerRolloutConfig, ServerRuntimeConfig, ServerWebhookConfig, ToolConfig,
};

// --- Linker ---
pub use linker::{LinkResult, Linker, LinkerConfig};

// --- Error ---
pub use error::DriverError;

// --- Runtime ---
pub use runtime::RuntimeExecutor;
