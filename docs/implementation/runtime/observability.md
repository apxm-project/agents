# Observability

APXM provides three observability layers -- structured tracing, a typed event system, and feature-gated metrics collection -- that can be independently enabled or disabled with zero overhead when off. For user-facing debugging instructions, see the [Debugging Guide](../../guides/debugging.md).

## Tracing

Structured tracing uses the `tracing` crate with domain-specific macros defined in `apxm-core/src/logging.rs`. Each macro targets a named subsystem so that `RUST_LOG` filters apply selectively:

| Macro | Target | Purpose |
|-------|--------|---------|
| `apxm_sched!` | `apxm::scheduler` | Worker lifecycle, ready queue, scheduling decisions |
| `apxm_op!` | `apxm::ops` | Operation dispatch/completion, with optional `worker = N` context |
| `apxm_llm!` | `apxm::llm` | LLM requests and responses |
| `apxm_token!` | `apxm::tokens` | Dataflow token production and consumption |
| `apxm_dag!` | `apxm::dag` | Artifact loading and DAG operations |
| `apxm_acp!` | `apxm::acp` | ACP protocol events (initialize, session, prompt) |
| `apxm_server!` | `apxm::server` | HTTP, A2A, agent registry |

General-purpose macros (`log_info!`, `log_warn!`, `log_error!`, `log_debug!`, `log_trace!`) wrap `tracing::event!` with a module tag parameter.

### Zero-overhead disabling

When the `no-trace` feature is enabled, all `apxm_*` macros compile to empty blocks. This is used in benchmark and production builds where tracing overhead is unacceptable.

### Filtering

```bash
RUST_LOG=apxm::scheduler=debug,apxm::llm=trace dekk apxm execute graph.apxm
```

## Event System

The `apxm-events` crate (a leaf dependency with no runtime coupling) defines a universal event envelope and typed payloads.

### ApxmEvent envelope

```rust
pub struct ApxmEvent {
    pub meta: EventMeta,      // seq, timestamp, trace_id, source
    pub payload: EventPayload,
}
```

`EventSource` distinguishes where events originate: `Backend(name)`, `Runtime`, `Session`, or `Server`.

### EventPayload -- 33 variants across 3 layers

**LLM Layer (9):** Token, Thought, ToolCall, LlmDone, Usage, Retry, Warning, Citation, ProviderEvent

**Runtime Layer (16):** OperationStart, OperationEnd, ToolStart, ToolEnd, PlanCreated, PlanStepStarted, PlanStepCompleted, MemoryRead, MemoryWrite, CheckpointSaved, CheckpointRestored, SchedulerDecision, GpuUtilization, TokenUsage, MemoizationHit, Error

**Session Layer (8):** ContextCompacted, ModelRerouted, Cancelled, LoopDetected, ContextWindowWarning, SessionStart, SessionEnd, TurnBoundary

The enum is `#[non_exhaustive]` and serde-tagged with `"kind"` so new variants can be added without breaking downstream consumers.

### EventBus

A `tokio::sync::broadcast`-based fan-out bus (default capacity 1024). Subscribers that fall behind receive a `Lagged(n)` error indicating how many events were missed.

```rust
let bus = EventBus::new();
let mut sub = bus.subscribe();
bus.publish(event)?;
let event = sub.recv().await?;
```

### EventEmitter trait

Components that produce events implement `EventEmitter`:

```rust
pub trait EventEmitter: Send + Sync {
    fn emit(&self, payload: EventPayload);
    fn emit_with_trace(&self, payload: EventPayload, trace_id: &str);
}
```

Implementors attach metadata (sequence numbers, timestamps, trace IDs) and route payloads to an `EventBus`.

## MetricsCollector

Defined in `apxm-runtime/src/observability/metrics.rs`, the `MetricsCollector` tracks scheduler overhead at nanosecond precision using atomic counters. It is feature-gated behind the `metrics` cargo feature.

### Overhead breakdown

Five scheduler phases are timed independently:

| Phase | What it measures |
|-------|-----------------|
| `ready_set_update` | Time updating the set of ready-to-fire nodes |
| `work_stealing` | Time spent in work-stealing across workers |
| `input_collection` | Time gathering inputs for an operation |
| `operation_dispatch` | Time dispatching an operation to a worker |
| `token_routing` | Time routing output tokens to downstream edges |

Each phase has cumulative nanosecond and count atomics, yielding per-operation averages via `overhead_breakdown_us()`.

### Parallelism measurement

- `max_concurrent_ops` -- highest observed in-flight operations (compare-exchange updated)
- `parallelism_samples` -- sampled every 16th operation to compute `average_parallelism()`
- Operation counters: `operations_executed`, `operations_failed`, `tokens_published`, `retries_attempted`

### Zero-overhead stub

When the `metrics` feature is disabled, `MetricsCollector` is a zero-sized `Copy` type where every method is `#[inline(always)]` and empty. The compiler eliminates all instrumentation calls entirely.

### timed! macro

```rust
let result = timed!(metrics, record_work_stealing, {
    state.work_stealing.steal_next()
});
```

When `metrics` is enabled, wraps the block with `Instant::now()` / `.elapsed()`. When disabled, compiles to just the inner block. An async variant `timed_async!` is also provided.

### SchedulerMetrics snapshot

`SchedulerMetrics::from_collector()` captures a point-in-time snapshot with overhead breakdown, parallelism stats, and operation counters. Serializable via `to_json()` for inclusion in `--emit-metrics` output.

## TokenAccountant

Defined in `apxm-runtime/src/executor/token_accounting.rs`, the `TokenAccountant` aggregates LLM token costs at three granularities:

| Scope | Key | What it tracks |
|-------|-----|---------------|
| Per-node | `node_id: u64` | Tokens consumed by each graph node |
| Per-flow | `flow_name: String` | Tokens aggregated across a named flow |
| Per-agent | `agent_name: String` | Tokens aggregated across an agent process |

Each scope tracks `input_tokens`, `output_tokens`, `total_tokens`, and `call_count`. The accountant is shared via `Arc<TokenAccountant>` so all LLM calls across a single execution roll up to one view.

`TokenAccountant::snapshot()` returns a `TokenAccountingSnapshot` that serializes to JSON for `--emit-metrics`.

## CLI Output Flags

### --emit-metrics

Writes a JSON file containing scheduler metrics (`SchedulerMetrics`) and token accounting (`TokenAccountingSnapshot`). Produced by `dekk apxm execute` and `dekk apxm run`.

### --emit-diagnostics

Writes a JSON file with compiler statistics: total compilation time, per-pass metrics (name, duration, ops eliminated), DAG statistics. Produced by `dekk apxm compile`.

### --emit-session

Writes a complete session folder for reproducible execution records. See [Sessions](sessions.md) for the folder layout.

## Session Output

When `--emit-session <dir>` is specified, each execution writes a folder:

```
<dir>/<execution-id>/
  manifest.json       # execution metadata (graph name, timestamp, parameters)
  input.json           # the input graph as submitted
  results.json         # final output values for all nodes
  metrics.json         # SchedulerMetrics + TokenAccountingSnapshot
  events.jsonl         # all ApxmEvent envelopes, one per line
  node_statuses.json   # per-node status (success/failure, duration, retries)
```

This folder is self-contained: replaying or auditing an execution requires only the session folder contents.

## Related Documentation

- [Debugging Guide](../../guides/debugging.md) -- user-facing guide: `--trace` flag, `RUST_LOG` filters, session replay
- [Sessions](sessions.md) -- AAM checkpoints, SessionManager, ProcessTable, ACP lifecycle
- [Dataflow Scheduler](dataflow-scheduler.md) -- scheduler internals that produce the metrics captured here
