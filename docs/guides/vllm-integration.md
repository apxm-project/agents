# vLLM Integration Guide

**Graph-Aware Scheduling for APXM**

This guide shows how to integrate APXM with vLLM to enable:
- **Priority scheduling** based on critical path analysis
- **KV-cache pinning** for prefix reuse across graph nodes
- **Graph lifecycle management** for automatic cleanup
- **Metrics** for cache hit rates and pin effectiveness

---

## Table of Contents

1. [Architecture Overview](#architecture-overview)
2. [Installation](#installation)
3. [Quick Start](#quick-start)
4. [Configuration](#configuration)
5. [How It Works](#how-it-works)
6. [Monitoring & Metrics](#monitoring--metrics)
7. [Troubleshooting](#troubleshooting)
8. [Performance Tuning](#performance-tuning)

---

## Architecture Overview

APXM's vLLM integration consists of three layers:

```
┌─────────────────────────────────────────────────────────┐
│  APXM Runtime (Rust)                                    │
│  - GraphAwareVllmBackend                                │
│  - Injects graph hints into requests                    │
│  - Calls register/release endpoints                     │
└──────────────────┬──────────────────────────────────────┘
                   │ HTTP (OpenAI-compatible + extensions)
                   ▼
┌─────────────────────────────────────────────────────────┐
│  APXM vLLM Scheduler API (Python)                       │
│  - FastAPI endpoints (/v1/apxm/graphs/...)             │
│  - GraphAwareScheduler state management                 │
│  - KV-cache pin registry                                │
└──────────────────┬──────────────────────────────────────┘
                   │ Python API
                   ▼
┌─────────────────────────────────────────────────────────┐
│  vLLM Server                                            │
│  - Scheduler uses APXM priority hints                   │
│  - KV-cache manager respects pin policies               │
│  - Standard OpenAI-compatible API                       │
└─────────────────────────────────────────────────────────┘
```

**Key Components:**

1. **GraphAwareVllmBackend** (Rust): Wraps OpenAI-compatible vLLM endpoint, injects APXM hints
2. **GraphAwareScheduler** (Python): Maintains graph metadata, pin registry, metrics
3. **REST API** (Python/FastAPI): HTTP endpoints for graph registration and management

---

## Installation

### 1. Install the APXM vLLM Extension

```bash
cd crates/apxm-backends/python
pip install -e .
```

This installs:
- `apxm_vllm` Python package
- `apxm-vllm-server` command-line tool

### 2. Set Up vLLM

Install vLLM (standard installation):

```bash
pip install vllm
```

**Note**: Full graph-aware KV pinning requires a vLLM fork with APXM extensions (Phase 3 from the strategy doc). For now, priority scheduling and hint pass-through work with stock vLLM.

---

## Quick Start

### 1. Start the APXM Scheduler API

```bash
apxm-vllm-server --host 0.0.0.0 --port 8001
```

Or programmatically:

```python
from apxm_vllm.api import create_apxm_app
import uvicorn

app = create_apxm_app()
uvicorn.run(app, host="0.0.0.0", port=8001)
```

The API will be available at `http://localhost:8001` with these endpoints:

- `POST /v1/apxm/graphs/register` — Register a graph
- `GET /v1/apxm/graphs/{graph_id}/priority/{node_id}` — Get node priority
- `DELETE /v1/apxm/graphs/{graph_id}` — Release graph
- `GET /v1/apxm/metrics` — Get scheduler metrics
- `GET /health` — Health check

### 2. Start vLLM

```bash
python -m vllm.entrypoints.openai.api_server \
  --model meta-llama/Llama-3.1-8B-Instruct \
  --host 0.0.0.0 \
  --port 8000
```

### 3. Configure APXM Backend

Add a vLLM backend to APXM:

```bash
apxm backend add vllm-local \
  --type local \
  --protocol vllm \
  --endpoint http://localhost:8000

apxm backend add-model vllm-local meta-llama/Llama-3.1-8B-Instruct
```

Verify:

```bash
apxm backend list
apxm backend test vllm-local
```

### 4. Run a Graph

Create a simple graph (`examples/vllm-test.apxm`):

```json
{
  "name": "vllm-test",
  "nodes": [
    {
      "id": 1,
      "name": "analyze_code",
      "op": "ASK",
      "attributes": {
        "template_str": "Analyze this code for security issues: {input}",
        "backend": "vllm-local",
        "model": "meta-llama/Llama-3.1-8B-Instruct"
      },
      "metadata": {
        "priority": 0
      }
    },
    {
      "id": 2,
      "name": "suggest_fixes",
      "op": "ASK",
      "attributes": {
        "template_str": "Based on this analysis: {1}, suggest fixes.",
        "backend": "vllm-local",
        "model": "meta-llama/Llama-3.1-8B-Instruct"
      },
      "metadata": {
        "priority": 0
      }
    }
  ],
  "edges": [
    {"from": 1, "to": 2, "dependency": "Data"}
  ],
  "parameters": [],
  "metadata": {}
}
```

Execute:

```bash
apxm execute examples/vllm-test.apxm --emit-session --emit-metrics metrics.json
```

---

## Configuration

### Backend Configuration

The vLLM backend is configured via `~/.apxm/config.toml`:

```toml
[[backends]]
name = "vllm-prod"
type = "cloud"
protocol = "vllm"
endpoint = "https://vllm.example.com"

[backends.config]
api_key = "sk-..."
base_url = "https://vllm.example.com"
default_pin_ttl_ms = 60000  # 60 seconds
enable_graph_registration = true
```

**Configuration Options:**

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `base_url` | string | `http://localhost:8000` | vLLM server URL |
| `api_key` | string | `""` | API key (if required) |
| `default_pin_ttl_ms` | int | `30000` | Default KV-cache pin TTL in ms |
| `enable_graph_registration` | bool | `true` | Auto-register graphs before execution |
| `scheduler_api_url` | string | `http://localhost:8001` | APXM scheduler API URL |

### Scheduler Configuration

The Python scheduler can be configured when creating the app:

```python
from apxm_vllm import GraphAwareScheduler
from apxm_vllm.api import create_apxm_app

scheduler = GraphAwareScheduler(
    default_pin_ttl_ms=60_000  # 60 seconds
)

app = create_apxm_app(scheduler=scheduler)
```

---

## How It Works

### Graph Registration (Automatic)

When APXM executes a graph with a vLLM backend:

1. **Before first request**: `GraphAwareVllmBackend.register_graph()` sends metadata to the scheduler API
2. **Graph metadata includes**:
   - Graph ID and execution ID
   - Per-node specs (priority, downstream dependencies, reuse groups)
   - Critical path length and max parallelism
   - Default pin TTL

**Example registration payload:**

```json
{
  "graph_id": "dag-123",
  "execution_id": "exec-2026-04-07-001",
  "critical_path_length": 4,
  "node_count": 8,
  "max_parallelism": 3,
  "default_pin_ttl_ms": 30000,
  "nodes": [
    {
      "node_id": 1,
      "node_name": "plan",
      "is_critical_path": true,
      "priority_class": "critical_path",
      "downstream_nodes": [2, 3],
      "reuse_group": "shared-context"
    }
  ]
}
```

### Request Hints (Per-Request)

Each LLM request includes APXM hints in `extra_body.apxm`:

```json
{
  "model": "meta-llama/Llama-3.1-8B-Instruct",
  "messages": [...],
  "priority": 0,
  "extra_body": {
    "apxm": {
      "schema_version": 1,
      "graph_id": "dag-123",
      "execution_id": "exec-2026-04-07-001",
      "node_id": 5,
      "node_name": "review_code",
      "priority_class": "critical_path",
      "downstream_nodes": [6, 7],
      "reuse_group": "pr-diff:shared-prefix",
      "pin_policy": {
        "mode": "prefix",
        "ttl_ms": 30000
      },
      "compiler_hints": {
        "shared_prefix_est_tokens": 2048,
        "warmup_candidate": true
      }
    }
  }
}
```

**Priority Mapping:**

| Priority Class | vLLM Priority | Use Case |
|----------------|---------------|----------|
| `critical_path` | 0 | User-visible results on critical path |
| `critical_path_non_interactive` | 2 | Background critical work |
| `normal` / `parallel` | 5 | Standard parallel branches |
| `speculative` | 10 | Speculative execution |
| `background` | 15 | Best-effort background tasks |

### KV-Cache Pinning

When a request completes with `pin_policy.mode = "prefix"`:

1. **Scheduler creates PinHandle**: Stores KV block IDs and expiry timestamp
2. **Indexed by**:
   - `(graph_id, node_id)` for direct upstream lookup
   - `reuse_group` for shared-prefix fan-out
3. **Downstream requests check for pins**: Before allocating new KV blocks
4. **Pin consumed or expires**: Blocks released back to vLLM

**Memory Pressure Policy** (from strategy doc §6.7):

| KV Usage | Action |
|----------|--------|
| < 85% | Allow all pins |
| 85-92% | Allow only critical-path pins |
| 92-97% | Downgrade new pins to normal cache |
| > 97% | Release non-critical pins immediately |

### Graph Cleanup

When a graph finishes (success, failure, or cancellation):

```python
# Rust backend calls:
await backend.release_graph(graph_id)

# Scheduler releases:
# - All pins for this graph
# - Graph metadata
# - Returns stats (released_handles, released_blocks)
```

---

## Monitoring & Metrics

### Scheduler Metrics

Query the metrics endpoint:

```bash
curl http://localhost:8001/v1/apxm/metrics
```

**Response:**

```json
{
  "metrics": {
    "total_registrations": 15,
    "total_releases": 12,
    "active_pins": 8,
    "pinned_blocks": 342,
    "pin_hits": 45,
    "pin_misses": 7,
    "pin_expirations": 3,
    "memory_pressure_releases": 0
  }
}
```

**Key Metrics:**

- **pin_hits / pin_misses**: Cache reuse effectiveness
- **active_pins**: Current memory overhead
- **pin_expirations**: TTL too short (increase if high)
- **memory_pressure_releases**: Server under pressure (tune thresholds)

### APXM Runtime Metrics

When running with `--emit-metrics`:

```bash
apxm execute graph.apxm --emit-metrics metrics.json
```

The metrics file includes:

- Total execution time
- Per-node latency
- LLM token counts
- Cache hit rates (if available from backend)

### Logging

Enable debug logging for the scheduler:

```python
import logging
logging.basicConfig(level=logging.DEBUG)
```

Or via environment variable:

```bash
export LOG_LEVEL=DEBUG
apxm-vllm-server
```

---

## Troubleshooting

### Problem: Hints not reaching vLLM

**Symptoms**: Requests execute but no priority/pin behavior

**Check**:

1. Verify backend protocol is `vllm`:
   ```bash
   apxm backend list | grep vllm
   ```

2. Inspect request JSON (add debug logging in `GraphAwareVllmBackend`):
   ```rust
   tracing::debug!("Request body: {:?}", serde_json::to_string_pretty(&body));
   ```

3. Check vLLM server logs for `extra_body.apxm` parsing

### Problem: Graph registration fails

**Symptoms**: `POST /v1/apxm/graphs/register` returns 4xx/5xx

**Solutions**:

1. Check scheduler API is running:
   ```bash
   curl http://localhost:8001/health
   ```

2. Verify graph metadata schema:
   ```bash
   curl -X POST http://localhost:8001/v1/apxm/graphs/register \
     -H "Content-Type: application/json" \
     -d @test-graph-meta.json
   ```

3. Check logs for validation errors

### Problem: Pin hit rate too low

**Symptoms**: `pin_misses >> pin_hits`

**Solutions**:

1. **Increase TTL**: Downstream requests arriving after pin expiry
   ```toml
   default_pin_ttl_ms = 60000  # Increase to 60s
   ```

2. **Check reuse_group alignment**: Nodes that share prefixes should have same `reuse_group`

3. **Verify prompt structure**: Shared context must be at the start (see Prompt Canonicalization in strategy doc §5.3)

### Problem: Memory pressure releases

**Symptoms**: `memory_pressure_releases` increasing

**Solutions**:

1. **Reduce pin TTL**: Shorter pins = less memory overhead
   ```toml
   default_pin_ttl_ms = 15000  # Reduce to 15s
   ```

2. **Pin only critical path**: Disable pinning for non-critical nodes

3. **Increase vLLM KV-cache size**: More memory available for pins
   ```bash
   --gpu-memory-utilization 0.95  # vLLM flag
   ```

---

## Performance Tuning

### Optimizing for Latency

**Goal**: Minimize end-to-end graph execution time

1. **Enable pinning for critical path**:
   ```rust
   pin_policy: PinPolicy::prefix(30_000)  // 30s TTL
   ```

2. **Use reuse groups for fan-out**:
   - Nodes 2, 3, 4 all read from node 1
   - Set `reuse_group = "analysis-context"` on all

3. **Tune priority classes**:
   - Critical path: `priority_class = "critical_path"`
   - Speculative: `priority_class = "speculative"`

### Optimizing for Throughput

**Goal**: Maximize concurrent request processing

1. **Disable pinning for high-throughput workloads**:
   ```rust
   pin_policy: PinPolicy::none()
   ```

2. **Use parallel priority**:
   ```rust
   priority_class: Some("parallel".to_string())
   ```

3. **Short TTLs to free memory faster**:
   ```toml
   default_pin_ttl_ms = 10000  # 10s
   ```

### Benchmarking

Measure the impact of graph-awareness:

```bash
# Baseline (no vLLM extensions)
apxm execute graph.apxm --emit-metrics baseline.json

# With graph-awareness
apxm execute graph.apxm --emit-metrics optimized.json

# Compare
apxm analyze --compare baseline.json optimized.json
```

**Expected Improvements** (from strategy doc §11.2):

- **Priority export**: Better critical-path completion under contention
- **Prompt shaping**: 20-60% less repeated prefill on fan-out
- **KV pinning**: Near-deterministic downstream reuse within TTL

---

## Advanced: Manual Graph Registration

For custom workflows, you can register graphs manually:

```python
import httpx

metadata = {
    "graph_id": "custom-dag",
    "execution_id": "run-123",
    "nodes": [
        {
            "node_id": 1,
            "node_name": "plan",
            "is_critical_path": True,
            "priority_class": "critical_path",
            "downstream_nodes": [2, 3],
        }
    ],
    "default_pin_ttl_ms": 45000,
}

response = httpx.post(
    "http://localhost:8001/v1/apxm/graphs/register",
    json=metadata
)

print(response.json())
# {"object": "apxm.graph.registration", "graph_id": "custom-dag", ...}
```

---

## See Also

- [Strategy Document](../strategy/09-VLLM-GRAPH-AWARENESS.md) — Full architectural plan
- [APXM Backends](backends.md) — Backend configuration guide
- [Graph Validation](debugging.md) — Debugging graph execution
- [vLLM Documentation](https://docs.vllm.ai/) — vLLM server setup

---

**Questions?** Check the [troubleshooting section](#troubleshooting) or file an issue.
