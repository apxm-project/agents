# vLLM Integration

> APXM integrates with vLLM for graph-aware inference scheduling. The runtime
> injects priority hints, KV-cache pin policies, and reuse-group metadata into
> every request so that vLLM can schedule critical-path nodes first and reuse
> prefilled prefixes across dependent nodes.

## Architecture

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
│  - Prometheus metrics exporter                          │
└─────────────────────────────────────────────────────────┘
```

**Key Components:**

1. **GraphAwareVllmBackend** (Rust) -- wraps the OpenAI-compatible vLLM endpoint and injects APXM hints into every request.
2. **GraphAwareScheduler** (Python) -- maintains graph metadata, the pin registry, and scheduling metrics.
3. **REST API** (Python/FastAPI) -- HTTP endpoints for graph registration and lifecycle management.

## Setup

### Docker Deployment

```bash
# Pull the APXM-enabled vLLM image
docker pull ghcr.io/apxm-project/vllm:latest

# Run with GPU support
docker run --gpus all \
  -p 8000:8000 \
  -v ~/.cache/huggingface:/root/.cache/huggingface \
  ghcr.io/apxm-project/vllm:latest \
  --model meta-llama/Llama-3.1-8B-Instruct \
  --enable-apxm \
  --prometheus-port 9090
```

**Key flags:**

| Flag | Description |
|------|-------------|
| `--enable-apxm` | Activate graph-aware scheduler |
| `--prometheus-port 9090` | Expose metrics endpoint |
| `--max-model-len 8192` | Context window size (default: model max) |
| `--gpu-memory-utilization 0.9` | Fraction of VRAM reserved for KV cache |

### Source Build

```bash
git clone https://github.com/apxm-project/vllm.git
cd vllm

conda create -n vllm python=3.11
conda activate vllm

pip install -e .
pip install prometheus-client

python -m vllm.entrypoints.openai.api_server \
  --model meta-llama/Llama-3.1-8B-Instruct \
  --enable-apxm \
  --prometheus-port 9090
```

Install the APXM scheduler sidecar:

```bash
cd crates/apxm-backends/python
pip install -e .

# Start the scheduler API
apxm-vllm-server --host 0.0.0.0 --port 8001
```

Verify the deployment:

```bash
curl http://localhost:8000/health
curl http://localhost:8000/v1/models
curl http://localhost:8000/v1/apxm/pins/stats
```

### Backend Registration

```bash
apxm backend add vllm-local \
  --type local \
  --protocol vllm \
  --endpoint http://localhost:8000

apxm backend add-model vllm-local meta-llama/Llama-3.1-8B-Instruct

apxm backend test vllm-local
```

## Configuration

Add the vLLM backend to `~/.apxm/config.toml`:

```toml
[[backends]]
name = "vllm-local"
type = "local"
protocol = "vllm"
endpoint = "http://localhost:8000"

[backends.config]
api_key = ""
scheduler_api_url = "http://localhost:8001"
default_pin_ttl_ms = 30000
enable_graph_registration = true
```

> See [config reference](../reference/config.md) for all options.

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `endpoint` | string | `http://localhost:8000` | vLLM server URL |
| `api_key` | string | `""` | API key (if required) |
| `default_pin_ttl_ms` | int | `30000` | Default KV-cache pin TTL in ms |
| `enable_graph_registration` | bool | `true` | Auto-register graphs before execution |
| `scheduler_api_url` | string | `http://localhost:8001` | APXM scheduler API URL |

The Python scheduler can also be configured programmatically:

```python
from apxm_vllm import GraphAwareScheduler
from apxm_vllm.api import create_apxm_app

scheduler = GraphAwareScheduler(default_pin_ttl_ms=60_000)
app = create_apxm_app(scheduler=scheduler)
```

## Graph Hints and KV Cache

APXM automatically injects graph metadata when using the `vllm` backend. No
code changes are needed.

### Graph Registration

Before the first request in a graph execution, `GraphAwareVllmBackend.register_graph()` sends metadata to the scheduler API. The payload includes the graph ID, execution ID, per-node specs (priority, downstream dependencies, reuse groups), critical path length, and max parallelism.

### Request Hints

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

### Priority Mapping

APXM maps `priority_class` to vLLM integer priorities:

| Priority Class | vLLM Priority | Use Case |
|----------------|---------------|----------|
| `critical_path` | 0 (highest) | User-visible results on critical path |
| `critical_path_non_interactive` | 2 | Background critical work |
| `normal` / `parallel` | 5 | Standard parallel branches |
| `speculative` | 10 | Speculative execution |
| `background` | 15 (lowest) | Best-effort background tasks |

Adjust priority strategy at compile time:

```bash
# Balanced priority across parallel branches
apxm compile graph.apxm --priority-strategy balanced

# Aggressive critical-path prioritization
apxm compile graph.apxm --priority-strategy aggressive
```

### Pin Lifecycle

When a request completes with `pin_policy.mode = "prefix"`:

1. **Scheduler creates a PinHandle** -- stores KV block IDs and expiry timestamp.
2. **Indexed by** `(graph_id, node_id)` for direct upstream lookup and by `reuse_group` for shared-prefix fan-out.
3. **Downstream requests check for pins** before allocating new KV blocks.
4. **Pin consumed or expires** -- blocks are released back to vLLM.

When a graph finishes (success, failure, or cancellation), the runtime calls `release_graph(graph_id)` which releases all pins, graph metadata, and returns stats (released handles, released blocks).

### Memory Pressure

The scheduler throttles pins based on KV-cache utilization:

| KV Usage | Action |
|----------|--------|
| < 85% | Allow all pins |
| 85-92% | Allow only critical-path pins |
| 92-97% | Downgrade new pins to normal cache |
| > 97% | Release non-critical pins immediately |

## Monitoring

### Prometheus Metrics

Expose metrics by starting vLLM with `--prometheus-port 9090`.

**APXM-specific metrics:**

| Metric | Description |
|--------|-------------|
| `apxm_pin_hits_total` | KV-cache prefix reuses (cache hits) |
| `apxm_pin_misses_total` | Cache misses (prefix not pinned) |
| `apxm_pin_hit_ratio` | Hit rate (0.0-1.0) |
| `apxm_active_pins` | Current number of pinned prefixes |
| `apxm_pinned_blocks` | KV blocks reserved by pins |
| `apxm_scheduling_delay_seconds` | Queue time for critical-path nodes |
| `apxm_graph_registrations_total` | Graphs registered |

**vLLM core metrics:**

| Metric | Description |
|--------|-------------|
| `vllm_request_queue_size` | Pending requests in scheduler |
| `vllm_gpu_cache_usage_percent` | KV cache utilization |
| `vllm_num_requests_running` | Concurrent executions |
| `vllm_time_to_first_token_seconds` | Prefill latency (P50/P95) |
| `vllm_time_per_output_token_seconds` | Decode throughput |

**Useful PromQL queries:**

Cache hit rate over 5 minutes:

```promql
rate(apxm_pin_hits_total[5m]) / (rate(apxm_pin_hits_total[5m]) + rate(apxm_pin_misses_total[5m]))
```

P95 scheduling delay for critical-path nodes:

```promql
histogram_quantile(0.95,
  rate(apxm_scheduling_delay_seconds_bucket{priority_class="critical_path"}[5m])
)
```

GPU memory pressure detection:

```promql
vllm_gpu_cache_usage_percent > 90
```

The scheduler API also exposes a JSON metrics endpoint:

```bash
curl http://localhost:8001/v1/apxm/metrics
```

### Grafana Dashboards

Import the pre-built APXM dashboard:

```bash
curl -O https://raw.githubusercontent.com/apxm-project/apxm/main/docs/assets/grafana-vllm-dashboard.json
# Grafana UI: Dashboards -> Import -> Upload JSON
```

Panels: request throughput, cache hit rate timeline, P95 latency by priority class, GPU memory utilization, active pin count.

## Production Deployment

### Multi-GPU (GPU)

Deploy on vendor GPU with GPU runtime and tensor parallelism:

```bash
docker run -d --name vllm-tp8 \
  --device=/dev/kfd --device=/dev/dri --group-add video \
  --ipc=host --cap-add=SYS_PTRACE --security-opt seccomp=unconfined \
  --shm-size=16g -p 8000:8000 \
  gpu/vllm:v0.14.0_amd_dev \
  python3 -m vllm.entrypoints.openai.api_server \
    --model Qwen/Qwen2.5-72B-Instruct \
    --tensor-parallel-size 8 \
    --host 0.0.0.0 --port 8000 \
    --enable-prefix-caching \
    --enable-auto-tool-choice \
    --tool-call-parser hermes \
    --trust-remote-code
```

**GPU runtime-specific flags:**

| Flag | Purpose |
|------|---------|
| `--device=/dev/kfd --device=/dev/dri` | GPU runtime GPU access |
| `--group-add video` | GPU access permissions |
| `--ipc=host` | Inter-process communication for RCCL |
| `--cap-add=SYS_PTRACE` | GPU runtime profiling support |
| `--security-opt seccomp=unconfined` | Prevents syscall blocking |
| `--shm-size=16g` | Shared memory for KV cache coordination across GPUs |

**Conservative single-GPU fallback** (if multi-GPU stability issues arise):

```bash
docker run -d --name vllm-stable \
  --device=/dev/kfd --device=/dev/dri --group-add video \
  --ipc=host --cap-add=SYS_PTRACE --security-opt seccomp=unconfined \
  --shm-size=16g -p 8000:8000 -e HIP_VISIBLE_DEVICES=0 \
  gpu/vllm:v0.14.0_amd_dev \
  python3 -m vllm.entrypoints.openai.api_server \
    --model Qwen/Qwen2.5-7B-Instruct \
    --host 0.0.0.0 --port 8000 \
    --enable-prefix-caching \
    --enable-auto-tool-choice \
    --tool-call-parser hermes \
    --trust-remote-code \
    --max-model-len 16384 \
    --gpu-memory-utilization 0.85
```

### Performance Tuning

**KV cache sizing** -- reserve 80-90% of VRAM:

```bash
--gpu-memory-utilization 0.85   # conservative
--gpu-memory-utilization 0.95   # aggressive (more room for pins)
```

**Batch size tuning** for multi-agent workloads:

```bash
--max-num-batched-tokens 8192   # more concurrent prefill
--max-num-seqs 256              # max concurrent sequences
```

Trade-off: higher batch size improves throughput but increases latency variance.

**Pin TTL tuning:**

- Latency-optimized: longer TTL (`60000`-`120000` ms), pin critical-path nodes, use `reuse_group` for fan-out.
- Throughput-optimized: shorter TTL (`10000`-`15000` ms) or disable pinning entirely to free memory faster.

Per-graph TTL override in graph metadata:

```json
{
  "name": "my-workflow",
  "metadata": {
    "apxm": { "default_pin_ttl_ms": 120000 }
  }
}
```

**Benchmarking:**

```bash
# Baseline (no optimizations)
apxm execute graph.apxm -O0 --emit-metrics baseline.json

# With graph-awareness
apxm execute graph.apxm --emit-metrics optimized.json
```

## Troubleshooting

### Hints not reaching vLLM

**Symptoms:** Requests execute but no priority/pin behavior.

1. Verify backend protocol is `vllm`: `apxm backend list | grep vllm`
2. Inspect request bodies via session tracing: `apxm execute graph.apxm --emit-session`
3. Check vLLM server logs for `extra_body.apxm` parsing.

### Graph registration fails

**Symptoms:** `POST /v1/apxm/graphs/register` returns 4xx/5xx.

1. Verify the scheduler API is running: `curl http://localhost:8001/health`
2. Confirm vLLM was started with `--enable-apxm`.
3. Check that the graph ID uses only alphanumeric characters and hyphens.
4. If duplicate registration, release the old graph first: `apxm execute graph.apxm --force-release`

### Low pin hit rate

**Symptoms:** `apxm_pin_hit_ratio < 0.3` or `pin_misses >> pin_hits`.

1. **Increase TTL** -- downstream requests may arrive after pin expiry.
2. **Check reuse_group alignment** -- nodes sharing prefixes must use the same `reuse_group`.
3. **Verify prompt structure** -- shared context must appear at the start of the prompt for prefix matching.

### Memory pressure releases

**Symptoms:** `memory_pressure_releases` or `apxm_pinned_blocks` climbing.

1. Reduce pin TTL to free memory faster.
2. Pin only critical-path nodes.
3. Increase vLLM KV-cache allocation: `--gpu-memory-utilization 0.95`

### High scheduling delay

**Symptoms:** `apxm_scheduling_delay_seconds` spikes.

1. Limit concurrent requests: `--max-num-seqs 128`
2. Disable speculative execution: `apxm execute graph.apxm --no-speculative`
3. Scale horizontally with tensor parallelism: `--tensor-parallel-size 2`

---

**See also:** [vLLM documentation](https://docs.vllm.ai/) | [Backend configuration](../reference/config.md)
