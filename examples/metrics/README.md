# Graph Metrics Example

This example shows the APXM metrics hierarchy:

- `graph_metrics.graph`: workflow totals.
- `graph_metrics.nodes`: node-owned operation and process metrics.
- `graph_metrics.aggregates`: derived rollups computed from node records.

Run the live graph with a configured ACP profile:

```sh
dekk apxm execute examples/metrics/spawn-agent-graph-metrics.json \
  --emit-session /tmp/apxm-metrics-session \
  --emit-metrics /tmp/apxm-graph-metrics.json
```

Inspect the graph-level metrics:

```sh
jq '.runtime.graph_metrics' /tmp/apxm-graph-metrics.json
```

Inspect per-node metrics:

```sh
find /tmp/apxm-metrics-session/nodes -name metrics.json -maxdepth 2 -print
```

`expected_graph_metrics.json` is a deterministic fixture for the same shape. It
is verified by the runtime test `documented_metrics_example_matches_fixture`,
so the documented node totals and graph aggregates cannot drift silently.

The key invariant is that spawned-agent activity is owned by the node that
caused it:

- The `SPAWN_AGENT` node records `process_spawns`.
- The `COMMUNICATE(acp)` node records `prompt_turns`.
- Graph totals and `aggregates.by_agent` are derived from those node records.

When a backend is APXM-aware, backend graph snapshots remain in the backend
metrics section. They can be correlated with node metrics, but they are not
owned by the runtime process metrics hierarchy.
