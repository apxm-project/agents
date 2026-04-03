---
title: "Memory Hierarchy"
description: "Rust implementation details for the three-tier memory hierarchy"
---

# Memory Hierarchy -- Implementation

For the formal memory model (tier semantics, cognitive rationale, comparative analysis), see [Memory in PXMs](../../pxm/memory.md).

This page covers the Rust backing-store choices and runtime lifecycle.

## Backing Stores

| Tier         | Implementation                                      | Concurrency                                    |
|--------------|-----------------------------------------------------|------------------------------------------------|
| **STM**      | `RwLock<HashMap<String, Value>>` (InMemoryBackend)  | RwLock-based (whole-store lock for writes, concurrent reads); scoped to execution |
| **LTM**      | Pluggable: InMemory, SQLite (WAL mode), or Redb     | Backend-dependent; SQLite: concurrent readers, single writer |
| **Episodic** | `VecDeque<EpisodicEntry>` with optional JSONL file persistence | RwLock-based append; readers acquire read lock  |

Each tier has an **independent lock** so a slow LTM write does not block a fast STM read.

## Runtime Lifecycle

1. **Execution starts** -- STM initialized empty; LTM and Episodic opened from disk.
2. **During execution** -- operations issue `QMEM`/`UMEM`. Episodic entries are recorded by handlers that opt in (not automatically at every operation boundary).
3. **Execution ends** -- STM dropped. LTM and Episodic persist (flushed to disk if backed by file storage).

## Additional Details

- **Scoped memory**: Keys prefixed with `__scope__/` are used for scope-level isolation within the memory subsystem.
- **Capacity limits**: STM and Episodic both support optional capacity limits (`max_capacity` on InMemoryBackend, `max_entries` on EpisodicMemory). When the limit is reached, oldest entries are evicted.
- **LTM backends**: LTM supports three backend options via `LtmBackend` enum: `Memory` (in-process HashMap), `Sqlite` (persistent, WAL mode), and `Redb` (persistent, embedded key-value store).
