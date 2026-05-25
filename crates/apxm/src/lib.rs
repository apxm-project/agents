//! APXM — Agent Programming eXecution Model.
//!
//! Top-level facade crate that re-exports all APXM subsystems.
//!
//! # Crate structure
//!
//! | Crate | Purpose |
//! |-------|---------|
//! | `apxm-core` | Shared types, errors, constants, execution graph primitives |
//! | `apxm-ais` | Agent Instruction Set — 100+ operation specifications |
//! | `apxm-compiler` | Multi-pass optimization pipeline for agent programs |
//! | `apxm-runtime` | Execution engine: memory system, scheduler, executor |
//! | `apxm-backends` | LLM providers (OpenAI, Anthropic, Google, Ollama), storage, prompts |

pub use apxm_ais as ais;
pub use apxm_backends as backends;
pub use apxm_compiler as compiler;
pub use apxm_core as core;
pub use apxm_runtime as runtime;

pub use apxm_core::error;
pub use apxm_core::types;
