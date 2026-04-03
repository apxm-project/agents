---
title: "Dataflow Scheduler"
description: "Rust implementation details for the A-PXM dataflow scheduler"
---

# Dataflow Scheduler -- Implementation

For the theoretical model (token-counting, O(1) readiness, work stealing, overhead breakdown, comparative analysis), see [Scheduling in PXMs](../../pxm/scheduling.md).

This page covers the Rust implementation specifics.

## Scheduler State

> **Simplified sketch** -- see `crates/apxm-runtime/src/scheduler/state.rs` for
> the full definition which includes work-stealing, concurrency control, metrics,
> and execution frames.

```rust
struct SchedulerState {
    // Immutable node data
    nodes: Arc<DashMap<NodeId, Arc<Node>>>,
    priorities: Arc<DashMap<NodeId, Priority>>,

    // Readiness tracking
    ready_set: ReadySet,
    tokens: Arc<DashMap<TokenId, TokenState>>,
    op_states: Arc<DashMap<NodeId, OpState>>,

    // Work-stealing scheduler + priority queue
    work_stealing: Arc<WorkStealingScheduler>,
    queue: Arc<PriorityQueue>,

    // Concurrency control
    concurrency: ConcurrencyControl,
    // ... coordination, promise tracking, etc.
}
```

`DashMap` provides sharded concurrent access so multiple executor threads can
retire tokens without contending on a single lock.

### Work-Stealing

The scheduler uses a 3-level steal hierarchy (implemented in
`scheduler/work_stealing.rs`):

1. **Local queue** -- worker pops from its own deque (fastest path).
2. **Global injectors** -- steal from priority-level injectors, high to low.
3. **Peer workers** -- round-robin steal from other workers' deques (slowest).

### Cost Budget Enforcement

Before execution begins, `enforce_cost_budget()` sums estimated latencies across
the DAG and rejects execution if the total exceeds the configured `max_cost`.
This happens before state construction, so no work is wasted.

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
critical-path length. At execution startup, `apply_goal_priorities()` projects
AAM goal priorities onto node scheduler priorities (using `max(compile_time,
goal_priority)`), but this projection happens once before workers start -- the
scheduler does not adjust priority dynamically during execution.
