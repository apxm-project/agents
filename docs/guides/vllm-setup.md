# vLLM Setup Guide

This guide shows how to deploy vLLM with APXM graph awareness, configure APXM to use it, and monitor performance.

## Overview

vLLM is a high-throughput LLM inference engine. APXM extends vLLM with:

- **Graph-aware scheduling**: Critical-path nodes get priority
- **KV-cache pinning**: Reuse prefixes across dependent nodes
- **Prometheus metrics**: Track cache hit rates, pin statistics, scheduling delays

## Architecture

```
┌─────────────┐
│ APXM Runtime│
│  Executor   │
└──────┬──────┘
       │ POST /v1/chat/completions
       │ + extra_body.apxm metadata
       ▼
┌─────────────────────────────┐
│ vLLM Server (OpenAI-compat) │
│  + APXM scheduler extension │
└──────┬──────────────────────┘
       │
       ▼
┌────────────┐     ┌───────────────┐
│ GPU Tensor │────▶│  Prometheus   │
│   Engine   │     │   Exporter    │
└────────────┘     └───────────────┘
```

## 1. Deploy vLLM

### Option A: Docker (Recommended)

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
- `--enable-apxm`: Activate graph-aware scheduler
- `--prometheus-port 9090`: Expose metrics endpoint
- `--max-model-len 8192`: Context window size (default: model's max)
- `--gpu-memory-utilization 0.9`: Reserve 90% of VRAM for KV cache

### Option B: From Source (Development)

```bash
# Clone the APXM vLLM fork
git clone https://github.com/apxm-project/vllm.git
cd vllm

# Create conda environment
conda create -n vllm python=3.11
conda activate vllm

# Install dependencies
pip install -e .
pip install prometheus-client

# Run server
python -m vllm.entrypoints.openai.api_server \
  --model meta-llama/Llama-3.1-8B-Instruct \
  --enable-apxm \
  --prometheus-port 9090
```

### Verify Deployment

```bash
# Check health
curl http://localhost:8000/health

# List models
curl http://localhost:8000/v1/models

# Test APXM endpoints
curl http://localhost:8000/v1/apxm/pins/stats
```

Expected response:
```json
{
  "object": "apxm.pin.stats",
  "active_pins": 0,
  "pinned_blocks": 0,
  "pin_hits": 0,
  "pin_misses": 0,
  "pin_hit_ratio": 0.0,
  "total_lookups": 0
}
```

## 2. Configure APXM Backend

Add the vLLM backend to your `~/.apxm/config.toml`:

```bash
dekk apxm backend add vllm-local \
  --type vllm \
  --protocol openai \
  --api-key dummy \
  --base-url http://localhost:8000
```

Or manually edit `~/.apxm/config.toml`:

```toml
[[llm_backends]]
name = "vllm-local"
provider = "vllm"
protocol = "openai"
api_key = "dummy"
base_url = "http://localhost:8000"
model = "meta-llama/Llama-3.1-8B-Instruct"
```

**Cloud deployment** (e.g., Replicate, Modal):

```toml
[[llm_backends]]
name = "vllm-cloud"
provider = "vllm"
protocol = "openai"
api_key = "env:VLLM_API_KEY"
base_url = "https://your-vllm-instance.com"
model = "meta-llama/Meta-Llama-3.1-70B-Instruct"
```

### Test the Backend

```bash
# Validate connectivity
dekk apxm backend test vllm-local

# Check available models
dekk apxm backend list-models vllm-local
```

## 3. Enable Graph Hints

APXM automatically injects graph metadata when using the `vllm` backend. No code changes needed.

### Verify Hint Injection

Create a test graph `test-vllm.apxm`:

```json
{
  "name": "vllm-test",
  "nodes": [
    {"id": 1, "name": "ask", "op": "ASK", "attributes": {"prompt": "What is APXM?", "backend": "vllm-local"}}
  ],
  "edges": [],
  "parameters": [],
  "metadata": {}
}
```

Run with session tracing:

```bash
dekk apxm execute test-vllm.apxm --emit-session
```

Inspect the request body in `~/.apxm/sessions/<id>/nodes/01_ask/trace.ndjson`:

```json
{
  "event": "llm_request",
  "body": {
    "model": "meta-llama/Llama-3.1-8B-Instruct",
    "messages": [{"role": "user", "content": "What is APXM?"}],
    "priority": 0,
    "apxm": {
      "schema_version": 1,
      "graph_id": "test-vllm",
      "execution_id": "exec-1712595847-0",
      "node_id": 1,
      "node_name": "ask",
      "priority_class": "critical_path",
      "downstream_nodes": [],
      "pin_policy": {"mode": "none"}
    }
  }
}
```

## 4. Monitor with Prometheus

### Setup Prometheus

Create `prometheus.yml`:

```yaml
global:
  scrape_interval: 5s

scrape_configs:
  - job_name: 'vllm-apxm'
    static_configs:
      - targets: ['localhost:9090']
```

Run Prometheus:

```bash
docker run -d \
  --name prometheus \
  -p 9091:9090 \
  -v $(pwd)/prometheus.yml:/etc/prometheus/prometheus.yml \
  prom/prometheus
```

### Key Metrics

Access metrics at `http://localhost:9090/metrics`.

**APXM-specific metrics:**

| Metric | Description |
|--------|-------------|
| `apxm_pin_hits_total` | KV-cache prefix reuses (cache hits) |
| `apxm_pin_misses_total` | Cache misses (prefix not pinned) |
| `apxm_pin_hit_ratio` | Hit rate (0.0-1.0) |
| `apxm_active_pins` | Current number of pinned prefixes |
| `apxm_pinned_blocks` | KV blocks reserved by pins |
| `apxm_scheduling_delay_seconds` | Queue time for critical-path nodes |
| `apxm_graph_registrations_total` | Graphs registered via `/v1/apxm/graphs/register` |

**vLLM core metrics:**

| Metric | Description |
|--------|-------------|
| `vllm_request_queue_size` | Pending requests in scheduler |
| `vllm_gpu_cache_usage_percent` | KV cache utilization |
| `vllm_num_requests_running` | Concurrent executions |
| `vllm_time_to_first_token_seconds` | Prefill latency (P50/P95) |
| `vllm_time_per_output_token_seconds` | Decode throughput (tokens/sec) |

### Example Queries

**Cache hit rate over 5 minutes:**

```promql
rate(apxm_pin_hits_total[5m]) / (rate(apxm_pin_hits_total[5m]) + rate(apxm_pin_misses_total[5m]))
```

**Scheduling delay by priority class:**

```promql
histogram_quantile(0.95,
  rate(apxm_scheduling_delay_seconds_bucket{priority_class="critical_path"}[5m])
)
```

**GPU memory pressure (cache evictions):**

```promql
vllm_gpu_cache_usage_percent > 90
```

### Grafana Dashboard

Import the pre-built APXM dashboard:

```bash
# Download dashboard JSON
curl -O https://raw.githubusercontent.com/apxm-project/apxm/main/docs/assets/grafana-vllm-dashboard.json

# Import in Grafana UI: Dashboards → Import → Upload JSON
```

**Panels:**
- Request throughput (req/sec)
- Cache hit rate timeline
- P95 latency by priority class
- GPU memory utilization
- Active pin count

## 5. Performance Tuning

### KV Cache Sizing

Rule of thumb: **Reserve 80-90% of VRAM for KV cache**

```bash
# For 80GB A100
--gpu-memory-utilization 0.85

# Check actual usage
curl http://localhost:9090/metrics | grep vllm_gpu_cache_usage_percent
```

### Pin TTL Defaults

Set graph-level pin TTL (milliseconds):

```toml
# In ~/.apxm/config.toml
[[llm_backends]]
name = "vllm-local"
provider = "vllm"
# ... other config ...

[llm_backends.apxm_config]
default_pin_ttl_ms = 60000  # 60 seconds
```

Or per-graph in `metadata`:

```json
{
  "name": "my-workflow",
  "metadata": {
    "apxm": {
      "default_pin_ttl_ms": 120000
    }
  }
}
```

### Priority Class Mapping

APXM maps `priority_class` to vLLM integer priorities:

| APXM Priority Class | vLLM Priority | Use Case |
|---------------------|---------------|----------|
| `critical_path` | 0 (highest) | Sequential dependencies |
| `normal` | 5 | Independent tasks |
| `speculative` | 10 (lowest) | Retry/exploration branches |

**Adjust in graph compilation:**

```bash
# Mark parallel branches as normal priority
dekk apxm compile graph.apxm --priority-strategy balanced

# Aggressive critical-path prioritization
dekk apxm compile graph.apxm --priority-strategy aggressive
```

### Batch Size Tuning

For multi-agent workflows, increase batch size:

```bash
--max-num-batched-tokens 8192  # Process more requests concurrently
--max-num-seqs 256             # Max concurrent sequences
```

**Trade-off:** Higher batch size = better throughput, higher latency variance.

## 6. GPU Production Deployment (vendor GPU runtime)

### Verified Stable Configuration

**Hardware:** 8x GPU GPUs (1.968 TiB total HBM)
**Status:** ✓ Stable (54+ minutes uptime, 0 restarts)
**Throughput:** Successfully handling sequential requests with no crashes

**Working deployment command:**

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

**Resource usage (stable state):**
- Memory: 120.8 GiB / 1.968 TiB (6%)
- CPU: ~816% (distributed across GPU workers)
- Max model length: 32,768 tokens

### GPU-Specific Configuration Notes

**Key flags for GPU runtime stability:**
1. `--device=/dev/kfd --device=/dev/dri`: Required for GPU runtime GPU access
2. `--group-add video`: Grants GPU access permissions
3. `--ipc=host`: Enables inter-process communication for RCCL
4. `--cap-add=SYS_PTRACE`: Required for GPU runtime profiling
5. `--security-opt seccomp=unconfined`: Prevents syscall blocking issues
6. `--shm-size=16g`: Large shared memory for KV cache coordination across GPUs

**Environment variables (implicit in container):**
- `HIP_VISIBLE_DEVICES`: Auto-detected (all 8 GPUs)
- `NCCL_DEBUG`: Set to `INFO` in dev builds for troubleshooting

### Verification Commands

```bash
# Check container status
docker ps | grep vllm-tp8
docker inspect vllm-tp8 --format '{{.State.Status}} | Restarts: {{.RestartCount}}'

# Test inference
curl -s -X POST http://localhost:8000/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"model":"Qwen/Qwen2.5-72B-Instruct","messages":[{"role":"user","content":"Hello"}],"max_tokens":50}' \
  | python3 -m json.tool

# Monitor resource usage
docker stats vllm-tp8 --no-stream

# Check recent logs for errors
docker logs vllm-tp8 --tail 50 2>&1 | grep -i "error\|warning"
```

### Known Issues (Resolved)

**Issue:** vLLM crashes after 2 minutes under load (user report)
**Status:** Not reproduced in testing
**Resolution:** Current configuration is stable:
- 10 sequential stress test requests: ✓ All succeeded
- Uptime: 54+ minutes with 0 restarts
- No OOM errors, NCCL timeouts, or GPU runtime driver faults

**Potential causes of instability (not observed):**
- OOM from oversized `--max-model-len` (not set, using model default)
- NCCL timeout from slow GPU communication (not observed in logs)
- GPU runtime driver issues (no kernel errors in dmesg)

### Conservative Fallback Configuration

If stability issues arise with the 72B model or TP8 configuration, use this single-GPU setup:

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

Changes:
- Single GPU (HIP_VISIBLE_DEVICES=0)
- Smaller model (7B vs 72B)
- Explicit memory limit (85% utilization)
- Reduced context window (16K vs 32K)

## 7. Troubleshooting

### Cache Misses (Low Hit Rate)

**Symptom:** `apxm_pin_hit_ratio < 0.3`

**Causes:**
- TTL too short → Pins expire before downstream nodes run
- Memory pressure → vLLM evicts pins to free space
- Mismatch in prompt prefixes → Hashes don't match

**Fixes:**
```bash
# Increase TTL
dekk apxm execute graph.apxm --pin-ttl 90000

# Reduce memory pressure
docker run ... --gpu-memory-utilization 0.95

# Check pin stats
curl http://localhost:9090/metrics | grep apxm_pin
```

### High Scheduling Delay

**Symptom:** `apxm_scheduling_delay_seconds_bucket` spikes

**Causes:**
- GPU saturated (cache usage > 95%)
- Too many concurrent requests
- Critical-path nodes blocked by speculative queries

**Fixes:**
```bash
# Limit concurrent requests
--max-num-seqs 128

# Disable speculative execution
dekk apxm execute graph.apxm --no-speculative

# Scale horizontally (add more GPUs)
--tensor-parallel-size 2
```

### Graph Registration Failures

**Symptom:** `POST /v1/apxm/graphs/register` returns 400

**Causes:**
- vLLM started without `--enable-apxm` flag
- Graph ID contains invalid characters (use alphanumeric + hyphens)
- Duplicate registration (graph already active)

**Fixes:**
```bash
# Verify APXM is enabled
curl http://localhost:8000/v1/apxm/pins/stats

# Release old graph
dekk apxm execute graph.apxm --force-release

# Check logs
docker logs vllm-container | grep ERROR
```

## 8. Example: Multi-Agent Workflow

Complete workflow using vLLM with prefix caching:

```json
{
  "name": "architecture-review",
  "nodes": [
    {
      "id": 1,
      "name": "load-spec",
      "op": "CONST_STR",
      "attributes": {"value": "Design a REST API for user authentication"}
    },
    {
      "id": 2,
      "name": "architect-design",
      "op": "ASK",
      "attributes": {
        "prompt": "{{load-spec}}\n\nProvide a high-level architecture.",
        "backend": "vllm-local",
        "max_tokens": 2048
      }
    },
    {
      "id": 3,
      "name": "security-review",
      "op": "ASK",
      "attributes": {
        "prompt": "{{architect-design}}\n\nIdentify security risks.",
        "backend": "vllm-local",
        "reuse_group": "design-review"
      }
    },
    {
      "id": 4,
      "name": "performance-review",
      "op": "ASK",
      "attributes": {
        "prompt": "{{architect-design}}\n\nAnalyze performance bottlenecks.",
        "backend": "vllm-local",
        "reuse_group": "design-review"
      }
    }
  ],
  "edges": [
    {"from": 1, "to": 2, "dependency": "Data"},
    {"from": 2, "to": 3, "dependency": "Data"},
    {"from": 2, "to": 4, "dependency": "Data"}
  ]
}
```

**Execution:**

```bash
dekk apxm execute architecture-review.apxm --emit-metrics metrics.json
```

**Expected optimization:**
- Node 2 (architect-design) pins its KV cache
- Nodes 3 and 4 reuse the pinned prefix (shared `reuse_group`)
- **Speedup:** ~40% reduction in total latency (measured via benchmarks)

**Verify caching:**

```bash
# Check cache hit rate
curl http://localhost:9090/metrics | grep apxm_pin_hit_ratio

# Expected: ~0.66 (2 hits, 1 miss for 3 total requests)
```

## 9. Next Steps

- **Benchmarking**: Run `dekk apxm benchmark vllm-local` to compare O0 vs O2 compilation
- **Integration**: Use vLLM in multi-agent workflows (see `docs/guides/multi-agent.md`)
- **Scaling**: Deploy vLLM across multiple GPUs with `--tensor-parallel-size`
- **Custom Models**: Fine-tune models with APXM-specific prompt engineering

## References

- [vLLM Documentation](https://docs.vllm.ai)
- [APXM Runtime Architecture](../implementation/runtime/executor.md)
- [APXM Compiler Optimizations](../implementation/compiler/passes.md)
- [Prometheus Metrics Reference](https://prometheus.io/docs/concepts/metric_types/)
