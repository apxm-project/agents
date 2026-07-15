//! Process-global per-capability write lock.
//!
//! Shared by BOTH parallelism engines so same-name write tools serialize
//! regardless of how they run:
//! - the in-ASK parallel tool loop (`handlers/llm/tool_dispatch.rs`), where the
//!   model emits a batch of tool calls in one turn, and
//! - the graph/dataflow path (`handlers/inv_cap.rs`), where the scheduler runs
//!   independent `inv_cap` nodes concurrently (it serializes by data dependency
//!   only, never by tool identity).
//!
//! Read-only tools never lock; a write tool acquires the write side of its
//! per-name `RwLock`, so concurrent writes to the SAME capability serialize
//! while distinct tools and all reads run free.

use std::sync::{Arc, LazyLock};
use tokio::sync::RwLock;

static TOOL_WRITE_LOCKS: LazyLock<dashmap::DashMap<String, Arc<RwLock<()>>>> =
    LazyLock::new(dashmap::DashMap::new);

/// Get (or create) the write lock for a capability name.
pub fn write_lock_for_tool(name: &str) -> Arc<RwLock<()>> {
    TOOL_WRITE_LOCKS
        .entry(name.to_string())
        .or_insert_with(|| Arc::new(RwLock::new(())))
        .clone()
}

/// Prune an idle lock entry (no in-flight users) to bound the map size.
pub fn release_write_lock_if_idle(name: &str, lock: &Arc<RwLock<()>>) {
    TOOL_WRITE_LOCKS.remove_if(name, |_, current| {
        Arc::ptr_eq(current, lock) && Arc::strong_count(current) == 2
    });
}
