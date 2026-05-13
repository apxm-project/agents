//! Dispatch IR — internal contract that carries compiled graph intent
//! from the APXM runtime to graph-aware inference backends.
//!
//! This module is internal-only. It is **not** re-exported from the crate
//! root and is **not** part of any public ABI. The governing design note is
//! `.apxm/docs/design/dispatch-ir.md`, which gates promotion to a public
//! contract on (1) measured benchmark proof and (2) a serious second
//! backend.
//!
//! See `.apxm/docs/design/agentic-infrastructure-final.md` for the
//! ownership boundary between APXM (graph semantics) and graph-aware
//! inference backends (token execution).

// The Dispatch IR is intentionally not yet consumed by any production code path.
// It is being introduced as the typed envelope that the vLLM adapter (and a
// future second backend) will lower to. See `.apxm/docs/design/dispatch-ir.md`.
#[allow(dead_code)]
pub mod v1;
