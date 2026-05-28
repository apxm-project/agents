//! APxM Driver - Configuration, linker, and orchestration
//!
//! This crate consolidates configuration and orchestration:
//!
//! - **`config`**: TOML-based configuration (`~/.apxm/config.toml`)
//! - **`linker`**: High-level linker orchestrating compiler/runtime
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
//!   │  TOML    │          │Orchestr. │          │  Exec    │
//!   │ Parsing  │          │  Logic   │          │  DAGs    │
//!   └──────────┘          └──────────┘          └──────────┘
//! ```

pub mod cache;
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
    HookEvent, RunEventsConfig, ServerConfig, ServerInferenceConfig, ServerRuntimeConfig,
    ToolConfig,
};

// --- Linker ---
pub use linker::{LinkResult, Linker, LinkerConfig};

// --- Error ---
pub use error::DriverError;

// --- Runtime ---
pub use runtime::RuntimeExecutor;
