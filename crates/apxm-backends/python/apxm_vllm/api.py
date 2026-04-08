"""FastAPI REST endpoints for APXM graph-aware vLLM integration.

This module provides HTTP endpoints for:
- Graph registration (POST /v1/apxm/graphs/register)
- Priority queries (GET /v1/apxm/graphs/{graph_id}/priority/{node_id})
- Graph release (DELETE /v1/apxm/graphs/{graph_id})
- Metrics (GET /v1/apxm/metrics)
"""

from typing import Optional
from fastapi import FastAPI, HTTPException, status
from pydantic import BaseModel, Field
from .graph_scheduler import GraphAwareScheduler


class NodeSpec(BaseModel):
    """Per-node specification for graph registration."""

    node_id: int
    node_name: Optional[str] = None
    estimated_prompt_tokens: Optional[int] = None
    downstream_nodes: list[int] = Field(default_factory=list)
    priority_class: Optional[str] = None
    reuse_group: Optional[str] = None
    is_critical_path: bool = False


class GraphRegisterRequest(BaseModel):
    """Request body for POST /v1/apxm/graphs/register."""

    graph_id: str
    execution_id: Optional[str] = None
    critical_path_length: Optional[int] = None
    node_count: Optional[int] = None
    max_parallelism: Optional[int] = None
    nodes: list[NodeSpec] = Field(default_factory=list)
    default_pin_ttl_ms: int = 30_000


class GraphRegisterResponse(BaseModel):
    """Response for graph registration."""

    object: str = "apxm.graph.registration"
    graph_id: str
    execution_id: Optional[str]
    registered_nodes: int


class GraphReleaseResponse(BaseModel):
    """Response for graph release."""

    object: str = "apxm.graph.release"
    graph_id: str
    released_handles: int
    released_blocks: int


class PriorityResponse(BaseModel):
    """Response for priority query."""

    graph_id: str
    node_id: int
    priority: int
    priority_class: Optional[str] = None


class MetricsResponse(BaseModel):
    """Response for metrics endpoint."""

    metrics: dict


class PinCreateRequest(BaseModel):
    """Request body for POST /v1/apxm/pins."""

    graph_id: str
    node_id: int
    reuse_group: Optional[str] = None
    ttl_ms: int = 30_000


class PinCreateResponse(BaseModel):
    """Response for pin creation."""

    object: str = "apxm.pin.created"
    graph_id: str
    node_id: int
    reuse_group: Optional[str]
    request_id: str
    ttl_ms: int
    expiry_ts: float


class PinStatsResponse(BaseModel):
    """Response for pin statistics endpoint."""

    object: str = "apxm.pins.stats"
    active_pins: int
    pinned_blocks: int
    pin_hits: int
    pin_misses: int
    pin_hit_ratio: float
    pin_expirations: int
    memory_pressure_releases: int
    total_lookups: int


def create_apxm_app(scheduler: Optional[GraphAwareScheduler] = None) -> FastAPI:
    """Create FastAPI app with APXM graph management endpoints.

    Args:
        scheduler: Optional existing scheduler instance (creates new if None)

    Returns:
        Configured FastAPI application
    """
    app = FastAPI(
        title="APXM vLLM Graph-Aware Scheduler",
        version="0.1.0",
        description="REST API for APXM graph metadata and KV-cache pinning",
    )

    # Global scheduler instance
    if scheduler is None:
        scheduler = GraphAwareScheduler()

    @app.post(
        "/v1/apxm/graphs/register",
        response_model=GraphRegisterResponse,
        status_code=status.HTTP_201_CREATED,
    )
    async def register_graph(request: GraphRegisterRequest) -> GraphRegisterResponse:
        """Register a graph's structure for scheduling hints.

        The scheduler uses this metadata to:
        - Assign request priorities based on critical path analysis
        - Make KV-cache pinning decisions for downstream reuse
        - Track graph lifecycle for cleanup

        Request body example:
        ```json
        {
          "graph_id": "dag-123",
          "execution_id": "exec-abc",
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
              "downstream_nodes": [2, 3]
            }
          ]
        }
        ```
        """
        # Convert to dict for scheduler
        metadata = request.model_dump()

        result = scheduler.register_graph(request.graph_id, metadata)

        return GraphRegisterResponse(**result)

    @app.get(
        "/v1/apxm/graphs/{graph_id}/priority/{node_id}",
        response_model=PriorityResponse,
    )
    async def get_priority(graph_id: str, node_id: int) -> PriorityResponse:
        """Get scheduling priority for a specific graph node.

        Priority values (lower = more urgent):
        - 0: Critical path, user-visible
        - 2: Critical path, non-interactive
        - 5: Normal / parallel
        - 10: Speculative
        - 15: Best-effort background

        Returns default priority (5) if graph or node not found.
        """
        priority = scheduler.get_priority(graph_id, node_id)

        # Try to get priority_class for response
        priority_class = None
        with scheduler._lock:
            meta = scheduler.registered_graphs.get(graph_id)
            if meta:
                node = meta.node_map.get(node_id)
                if node:
                    priority_class = node.priority_class

        return PriorityResponse(
            graph_id=graph_id,
            node_id=node_id,
            priority=priority,
            priority_class=priority_class,
        )

    @app.delete(
        "/v1/apxm/graphs/{graph_id}",
        response_model=GraphReleaseResponse,
    )
    async def release_graph(graph_id: str) -> GraphReleaseResponse:
        """Release a graph's KV-cache pins and metadata.

        Call this when:
        - Graph execution completes successfully
        - Graph execution is cancelled
        - Graph execution fails and should be cleaned up

        All pinned KV-cache blocks for this graph are freed.
        """
        result = scheduler.release_graph(graph_id)
        return GraphReleaseResponse(**result)

    @app.get("/v1/apxm/metrics", response_model=MetricsResponse)
    async def get_metrics() -> MetricsResponse:
        """Get current scheduler metrics.

        Returns metrics including:
        - total_registrations: Graphs registered (cumulative)
        - total_releases: Graphs released (cumulative)
        - active_pins: Current number of active pins
        - pinned_blocks: Current number of pinned KV blocks
        - pin_hits: Successful prefix reuse (cumulative)
        - pin_misses: Failed prefix lookups (cumulative)
        - pin_expirations: Pins expired due to TTL (cumulative)
        - memory_pressure_releases: Pins released under pressure (cumulative)
        """
        metrics = scheduler.get_metrics()
        return MetricsResponse(metrics=metrics)

    @app.post(
        "/v1/apxm/pins",
        response_model=PinCreateResponse,
        status_code=status.HTTP_201_CREATED,
    )
    async def create_pin(request: PinCreateRequest) -> PinCreateResponse:
        """Create a KV-cache pin for prefix reuse.

        Call this after an LLM request completes to pin its KV-cache blocks
        for downstream reuse within a TTL.

        Request body example:
        ```json
        {
          "graph_id": "dag-123",
          "node_id": 5,
          "reuse_group": "pr-diff:shared-prefix",
          "ttl_ms": 30000
        }
        ```

        The pin will be automatically released when:
        - A downstream request consumes it
        - TTL expires
        - Graph is released (DELETE /v1/apxm/graphs/{graph_id})
        - Memory pressure policy triggers cleanup
        """
        result = scheduler.pin_prefix(
            graph_id=request.graph_id,
            node_id=request.node_id,
            reuse_group=request.reuse_group,
            ttl_ms=request.ttl_ms,
        )
        return PinCreateResponse(**result)

    @app.get("/v1/apxm/pins/stats", response_model=PinStatsResponse)
    async def get_pin_stats() -> PinStatsResponse:
        """Get detailed pin statistics.

        Returns:
        - active_pins: Current number of active pins
        - pinned_blocks: Current number of pinned KV blocks
        - pin_hits: Successful prefix reuse (cumulative)
        - pin_misses: Failed prefix lookups (cumulative)
        - pin_hit_ratio: Ratio of hits to total lookups
        - pin_expirations: Pins expired due to TTL (cumulative)
        - memory_pressure_releases: Pins released under pressure (cumulative)
        - total_lookups: Total pin lookup attempts
        """
        stats = scheduler.get_pin_stats()
        return PinStatsResponse(**stats)

    @app.get("/health")
    async def health_check():
        """Health check endpoint."""
        return {"status": "healthy", "service": "apxm-vllm-scheduler"}

    # Store scheduler on app for access from vLLM integration
    app.state.scheduler = scheduler

    return app


# Convenience function for standalone server
def run_server(host: str = "0.0.0.0", port: int = 8001):
    """Run the APXM API server standalone.

    Args:
        host: Host to bind to (default: 0.0.0.0)
        port: Port to bind to (default: 8001)
    """
    import uvicorn

    app = create_apxm_app()
    uvicorn.run(app, host=host, port=port)


if __name__ == "__main__":
    run_server()
