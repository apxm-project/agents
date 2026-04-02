# Debugging APXM

APXM provides structured, zero-overhead tracing across all subsystems. When disabled (default), tracing compiles to nothing. When enabled, it emits structured events to stderr with thread IDs, targets, and timestamps.

## Enabling Tracing

### Via `--trace` flag (all subsystems at one level)

```bash
apxm --trace debug execute graph.json
apxm --trace trace agent test claude
```

### Via `RUST_LOG` (selective, works with dekk)

```bash
# ACP protocol messages only
RUST_LOG=apxm::acp=debug dekk apxm agent test claude

# Multiple subsystems at different levels
RUST_LOG=apxm::acp=debug,apxm::ops=trace dekk apxm execute graph.json

# Everything at trace level
RUST_LOG=apxm=trace dekk apxm execute graph.json
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
# See the full JSON-RPC handshake
RUST_LOG=apxm::acp=debug dekk apxm agent test claude
```

Output:
```
DEBUG apxm::acp: -> request method=initialize id=1
DEBUG apxm::acp: <- ok id=1
DEBUG apxm::acp: -> request method=session/new id=2
DEBUG apxm::acp: <- ok id=2
INFO  apxm::acp: session established agent="claude" session_id=...
```

If something fails, you'll see the error with full detail:
```
WARN  apxm::acp: <- error id=2 code=-32602 msg=Invalid params data={...}
```

## Example: Debugging Graph Execution

```bash
# Scheduler + operation tracing
RUST_LOG=apxm::ops=debug,apxm::scheduler=info dekk apxm execute graph.json
```

## Metrics

Runtime metrics are separate from tracing, enabled via the `metrics` feature:

```bash
dekk apxm execute graph.json --emit-metrics metrics.json
```

Metrics include scheduler overhead (ns), parallelism, work stealing, and per-operation timing.

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

The macros follow `tracing` conventions: first arg is level (`trace`, `debug`, `info`, `warn`, `error`), then structured fields, then message string.
