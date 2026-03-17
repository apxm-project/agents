---
title: "Dataflow Scheduler"
description: "Rust implementation details for the A-PXM dataflow scheduler"
---

# Dataflow Scheduler -- Implementation

For the theoretical model (token-counting, O(1) readiness, work stealing, overhead breakdown, comparative analysis), see [Scheduling in PXMs](../../pxm/scheduling.md).

This page covers the Rust implementation specifics.

## Scheduler State

```rust
struct SchedulerState {
    tokens: DashMap<TokenId, TokenState>,   // live token metadata
    ops: DashMap<NodeId, OpState>,          // pending counters + status
    ready: ReadySet,                        // lock-free set of runnable nodes
    queue: PriorityQueue<NodeId, Priority>, // scheduling order
}
```

`DashMap` provides sharded concurrent access so multiple executor threads can
retire tokens without contending on a single lock. Chosen over `RwLock<HashMap>`
because it eliminates reader-writer contention at the cost of slightly higher
per-key overhead -- a worthwhile trade at the concurrency levels A-PXM targets.

## Backpressure

If the ready set exceeds a configurable **high-water mark** the scheduler stops
accepting new external triggers, applying backpressure to upstream callers. This
prevents unbounded memory growth in bursty workloads.

The concurrency controller uses a semaphore-based **max inflight operation count**.
Workers block on permit acquisition when the limit is reached. A **watchdog**
monitors progress and diagnoses deadlocks if no operation completes within a
configurable timeout.

## Priority Policy

The compiler annotates each node with a static priority estimate based on
critical-path length. The scheduler uses it as the initial key and may adjust
dynamically based on observed latency.
