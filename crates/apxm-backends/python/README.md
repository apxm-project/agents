# APXM vLLM Graph-Aware Scheduler

Python extension for vLLM that adds APXM graph metadata awareness.

## Installation

```bash
pip install -e .
```

With test dependencies:

```bash
pip install -e ".[test]"
```

## Running the Scheduler API

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

## Running Tests

```bash
pytest
```

With coverage:

```bash
pytest --cov=apxm_vllm --cov-report=html
```

## Documentation

See [docs/guides/vllm-integration.md](../../../docs/guides/vllm-integration.md) for the full integration guide.

## API Endpoints

- `POST /v1/apxm/graphs/register` — Register graph metadata
- `GET /v1/apxm/graphs/{graph_id}/priority/{node_id}` — Get node priority
- `DELETE /v1/apxm/graphs/{graph_id}` — Release graph
- `GET /v1/apxm/metrics` — Get scheduler metrics
- `GET /health` — Health check

## Architecture

```
APXM Runtime (Rust)
    ↓ HTTP requests with extra_body.apxm hints
APXM Scheduler API (Python/FastAPI)
    ↓ Priority hints + pin decisions
vLLM Server
```

See [docs/strategy/09-VLLM-GRAPH-AWARENESS.md](../../../docs/strategy/09-VLLM-GRAPH-AWARENESS.md) for the full architecture.
