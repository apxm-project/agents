//! Dispatch IR — internal contract that carries compiled graph intent
//! from the APXM runtime to graph-aware inference backends.
//!
//! This module is internal-only. It is **not** re-exported from the crate
//! root and is **not** part of any public ABI. Promotion to a public contract
//! requires measured benchmark proof and a serious second backend.
//!
//! APXM owns graph semantics; graph-aware inference backends own token
//! execution.

// The Dispatch IR intentionally has no production consumer yet.
// It is being introduced as the typed envelope that the vLLM adapter (and a
// future second backend) will lower to.
pub mod v1;
