---
title: "Memory Hierarchy"
description: "Rust implementation details for the three-tier memory hierarchy"
---

# Memory Hierarchy -- Implementation

For the formal memory model (tier semantics, cognitive rationale, comparative analysis), see [Memory in PXMs](../../pxm/memory.md).

This page covers the Rust backing-store choices and runtime lifecycle.

## Backing Stores

| Tier         | Implementation                  | Concurrency                                    |
|--------------|---------------------------------|------------------------------------------------|
| **STM**      | `DashMap<String, Value>`        | Lock-free via sharding; scoped to execution    |
| **LTM**      | Embedded SQLite (WAL mode)      | Concurrent readers, single writer              |
| **Episodic** | Append-only log (WAL snapshots) | Single-writer append; readers snapshot via WAL  |

Each tier has an **independent lock** so a slow LTM write does not block a fast STM read.

## Runtime Lifecycle

1. **Execution starts** -- STM initialized empty; LTM and Episodic opened from disk.
2. **During execution** -- operations issue `QMEM`/`UMEM`. Episodic entries appended automatically at every operation boundary.
3. **Execution ends** -- STM dropped. LTM and Episodic flushed and remain on disk.
