//! Owner-local Execution Commit adapters for contract conformance.
//!
//! These adapters implement [`apxm_kernel::ExecutionCommitPort`] with
//! in-memory and single-directory filesystem backends. They are composition
//! roots for owner-local conformance only.
//!
//! They are **not** a hosted durable checkpoint/output service:
//! - no network listener, remote protocol, or multi-tenant topology;
//! - not wired into the runtime kernel by default; the kernel accepts an
//!   injected `ExecutionCommitPort` through its exact port bundle;
//! - durability beyond one process or one directory is supplied by a
//!   downstream-injected Port binding, not by an APXM hosted product.

mod filesystem;
mod memory;
mod store;

pub use filesystem::FilesystemExecutionCommit;
pub use memory::InMemoryExecutionCommit;
pub use store::{
    COMMIT_LOCAL_SCHEMA, CommitLocalError, CommitLocalRecord, CommitLocalStore, CommitLocalTuple,
    CommitRequestIdentity, MAX_COMMIT_RESULTS, MAX_OUTPUT_BYTES, MAX_OUTPUT_RECORDS,
    MAX_STORE_BYTES, MAX_TUPLE_BYTES, PreparedOutputRef, SessionOutputPreparation, StoredCommit,
    StoredOutput,
};
