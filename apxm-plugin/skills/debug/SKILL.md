---
name: debug
description: Debug agent workflow execution
user-invocable: true
---

# Debugging Agent Workflows

## Tracing

Enable trace output at different levels:

```bash
dekk apxm execute graph.air --trace info
dekk apxm execute graph.air --trace debug
```

### Environment-based tracing

```bash
RUST_LOG=apxm::scheduler=debug dekk apxm execute graph.air
RUST_LOG=apxm::ops=trace dekk apxm execute graph.air
```

### Tracing targets

| Target | What it traces |
|--------|---------------|
| `scheduler` | Task scheduling, parallelism decisions |
| `ops` | Individual operation execution |
| `llm` | LLM API calls and responses |
| `tokens` | Token counting and budget tracking |
| `dag` | DAG construction and traversal |
| `acp` | Agent Communication Protocol messages |
| `server` | HTTP server requests and responses |

## Metrics

Emit runtime metrics to a JSON file:

```bash
dekk apxm execute graph.air --emit-metrics metrics.json
```

## References

- [Debugging Guide](docs/guides/debugging.md)
