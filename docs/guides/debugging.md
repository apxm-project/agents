# Debugging APXM

APXM provides structured, zero-overhead tracing across all subsystems. When disabled (default), tracing compiles to nothing. When enabled, it emits structured events to stderr with thread IDs, targets, and timestamps.

## Enabling Tracing

### Via `--trace` flag (all subsystems at one level)

```bash
apxm --trace debug execute graph.apxm
apxm --trace trace agent test claude
```

### Via `RUST_LOG` (selective)

```bash
# ACP protocol messages only
RUST_LOG=apxm::acp=debug apxm agent test claude

# Multiple subsystems at different levels
RUST_LOG=apxm::acp=debug,apxm::ops=trace apxm execute graph.apxm

# Everything at trace level
RUST_LOG=apxm=trace apxm execute graph.apxm
```

## Tracing Targets

Each subsystem has a dedicated macro and target for filtering:

| Target | Macro | What it traces |
|--------|-------|----------------|
| `apxm::scheduler` | `apxm_sched!` | Worker lifecycle, ready queue, work stealing |
| `apxm::ops` | `apxm_op!` | Operation dispatch, completion, inputs |
| `apxm::llm` | `apxm_llm!` | LLM requests, token budgets, reasoning modes |
| `apxm::tokens` | `apxm_token!` | Token production, consumption, routing |
| `apxm::dag` | `apxm_dag!` | DAG loading, structure, entry/exit nodes |
| `apxm::acp` | `apxm_acp!` | ACP protocol: initialize, session, prompt, reverse requests |
| `apxm::server` | `apxm_server!` | HTTP endpoints, A2A tasks, agent registry |

## Example: Debugging an ACP Agent

```bash
RUST_LOG=apxm::acp=debug apxm agent test claude
```

Output:
```
DEBUG apxm::acp: -> request method=initialize id=1
DEBUG apxm::acp: <- ok id=1
DEBUG apxm::acp: -> request method=session/new id=2
DEBUG apxm::acp: <- ok id=2
INFO  apxm::acp: session established agent="claude" session_id=...
```

If something fails, the error appears with full detail:
```
WARN  apxm::acp: <- error id=2 code=-32602 msg=Invalid params data={...}
```

## Example: Debugging Graph Execution

```bash
RUST_LOG=apxm::ops=debug,apxm::scheduler=info apxm execute graph.apxm
```

This shows each operation as it dispatches and completes, along with scheduler-level events (worker starts, ready-queue depth, work-stealing events).

## Session Tracing

For full execution traces persisted to disk, use `--emit-session`:

```bash
apxm execute graph.apxm --emit-session
```

This creates a session directory with per-node workspaces, an NDJSON event stream, and a live progress snapshot. See [Session Output](../implementation/runtime/sessions.md) for the directory layout.

## Metrics

Runtime metrics are separate from tracing, enabled via `--emit-metrics`:

```bash
apxm execute graph.apxm --emit-metrics metrics.json
```

Metrics include scheduler overhead (ns), parallelism, work stealing, and per-operation timing. See [Runtime Observability](../implementation/runtime/observability.md) for the full metrics schema.

## Zero-Overhead Builds

For benchmarks or production, compile with `no-trace` to eliminate all tracing code:

```bash
cargo build -p apxm-cli --features driver,no-trace --release
```

## Using Macros in Code

All macros are defined in `apxm-core/src/logging.rs` and exported via `#[macro_export]`. To use in a crate:

```rust
use apxm_core::apxm_acp;

apxm_acp!(debug, method = %method, id = id, "-> request");
apxm_acp!(warn, code = err.code, msg = %err.message, "<- error");
```

The macros follow `tracing` conventions: first argument is level (`trace`, `debug`, `info`, `warn`, `error`), then structured fields, then message string.

---

## See Also

- [Session Output](../implementation/runtime/sessions.md) -- Session directory layout and replay
- [Runtime Observability](../implementation/runtime/observability.md) -- Metrics schema and collection
- [Dataflow Scheduler](../implementation/runtime/dataflow-scheduler.md) -- Scheduler internals traced by `apxm::scheduler`
- [AIS: Communication Ops](../implementation/ais/communication.md) -- ACP protocol details traced by `apxm::acp`
- [Getting Started](../getting-started/installation.md) -- `apxm doctor` for environment diagnostics
