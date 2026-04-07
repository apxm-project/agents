"""Tests for FastAPI REST endpoints."""

import pytest
from fastapi.testclient import TestClient
from apxm_vllm.api import create_apxm_app
from apxm_vllm.graph_scheduler import GraphAwareScheduler


@pytest.fixture
def client():
    """Create test client with fresh scheduler."""
    app = create_apxm_app()
    return TestClient(app)


@pytest.fixture
def scheduler():
    """Access the scheduler instance from the app."""
    app = create_apxm_app()
    return app.state.scheduler


def test_health_check(client):
    """Test health check endpoint."""
    response = client.get("/health")

    assert response.status_code == 200
    data = response.json()
    assert data["status"] == "healthy"
    assert data["service"] == "apxm-vllm-scheduler"


def test_register_graph(client):
    """Test POST /v1/apxm/graphs/register."""
    request_data = {
        "graph_id": "test-graph",
        "execution_id": "exec-123",
        "critical_path_length": 3,
        "node_count": 5,
        "nodes": [
            {
                "node_id": 1,
                "node_name": "plan",
                "is_critical_path": True,
                "priority_class": "critical_path",
                "downstream_nodes": [2, 3],
            },
            {
                "node_id": 2,
                "node_name": "execute",
                "is_critical_path": False,
                "priority_class": "parallel",
                "downstream_nodes": [],
            },
        ],
        "default_pin_ttl_ms": 45000,
    }

    response = client.post("/v1/apxm/graphs/register", json=request_data)

    assert response.status_code == 201
    data = response.json()
    assert data["object"] == "apxm.graph.registration"
    assert data["graph_id"] == "test-graph"
    assert data["execution_id"] == "exec-123"
    assert data["registered_nodes"] == 2


def test_register_graph_minimal(client):
    """Test graph registration with minimal data."""
    request_data = {
        "graph_id": "minimal-graph",
    }

    response = client.post("/v1/apxm/graphs/register", json=request_data)

    assert response.status_code == 201
    data = response.json()
    assert data["graph_id"] == "minimal-graph"
    assert data["registered_nodes"] == 0


def test_get_priority(client):
    """Test GET /v1/apxm/graphs/{graph_id}/priority/{node_id}."""
    # Register a graph first
    register_data = {
        "graph_id": "g1",
        "nodes": [
            {
                "node_id": 1,
                "priority_class": "critical_path",
                "is_critical_path": True,
                "downstream_nodes": [2],
            }
        ],
    }
    client.post("/v1/apxm/graphs/register", json=register_data)

    # Query priority
    response = client.get("/v1/apxm/graphs/g1/priority/1")

    assert response.status_code == 200
    data = response.json()
    assert data["graph_id"] == "g1"
    assert data["node_id"] == 1
    assert data["priority"] == 0  # critical_path
    assert data["priority_class"] == "critical_path"


def test_get_priority_default(client):
    """Test priority query for unknown graph returns default."""
    response = client.get("/v1/apxm/graphs/unknown/priority/1")

    assert response.status_code == 200
    data = response.json()
    assert data["priority"] == 5  # default


def test_release_graph(client):
    """Test DELETE /v1/apxm/graphs/{graph_id}."""
    # Register a graph
    register_data = {
        "graph_id": "g1",
        "nodes": [{"node_id": 1, "is_critical_path": True, "downstream_nodes": []}],
    }
    client.post("/v1/apxm/graphs/register", json=register_data)

    # Release it
    response = client.delete("/v1/apxm/graphs/g1")

    assert response.status_code == 200
    data = response.json()
    assert data["object"] == "apxm.graph.release"
    assert data["graph_id"] == "g1"
    assert "released_handles" in data
    assert "released_blocks" in data


def test_get_metrics(client):
    """Test GET /v1/apxm/metrics."""
    response = client.get("/v1/apxm/metrics")

    assert response.status_code == 200
    data = response.json()
    assert "metrics" in data

    metrics = data["metrics"]
    assert "total_registrations" in metrics
    assert "total_releases" in metrics
    assert "active_pins" in metrics
    assert "pinned_blocks" in metrics
    assert "pin_hits" in metrics
    assert "pin_misses" in metrics


def test_metrics_after_operations(client):
    """Test metrics reflect operations."""
    # Register 2 graphs
    client.post("/v1/apxm/graphs/register", json={"graph_id": "g1"})
    client.post("/v1/apxm/graphs/register", json={"graph_id": "g2"})

    # Release 1 graph
    client.delete("/v1/apxm/graphs/g1")

    # Check metrics
    response = client.get("/v1/apxm/metrics")
    metrics = response.json()["metrics"]

    assert metrics["total_registrations"] == 2
    assert metrics["total_releases"] == 1


def test_api_validation_error():
    """Test API validation for invalid request."""
    app = create_apxm_app()
    client = TestClient(app, raise_server_exceptions=False)

    # Missing required field
    response = client.post("/v1/apxm/graphs/register", json={})

    assert response.status_code == 422  # Validation error


def test_custom_scheduler():
    """Test API with custom scheduler instance."""
    custom_scheduler = GraphAwareScheduler(default_pin_ttl_ms=60_000)
    app = create_apxm_app(scheduler=custom_scheduler)
    client = TestClient(app)

    # Verify the custom scheduler is used
    assert app.state.scheduler.default_pin_ttl_ms == 60_000

    # Register a graph
    response = client.post(
        "/v1/apxm/graphs/register",
        json={"graph_id": "test", "default_pin_ttl_ms": 60_000},
    )

    assert response.status_code == 201

    # Check the graph was registered in our custom scheduler
    assert "test" in custom_scheduler.registered_graphs


def test_concurrent_requests(client):
    """Test concurrent API requests (simulated)."""
    import concurrent.futures

    def register_graph(graph_id):
        return client.post(
            "/v1/apxm/graphs/register",
            json={"graph_id": graph_id},
        )

    with concurrent.futures.ThreadPoolExecutor(max_workers=5) as executor:
        futures = [executor.submit(register_graph, f"g{i}") for i in range(10)]
        responses = [f.result() for f in futures]

    # All should succeed
    assert all(r.status_code == 201 for r in responses)

    # Check metrics
    response = client.get("/v1/apxm/metrics")
    metrics = response.json()["metrics"]
    assert metrics["total_registrations"] == 10


def test_release_nonexistent_graph(client):
    """Test releasing a graph that was never registered."""
    response = client.delete("/v1/apxm/graphs/nonexistent")

    # Should still succeed (idempotent)
    assert response.status_code == 200
    data = response.json()
    assert data["released_handles"] == 0
    assert data["released_blocks"] == 0


def test_priority_response_structure(client):
    """Test complete priority response structure."""
    # Register graph with node
    client.post(
        "/v1/apxm/graphs/register",
        json={
            "graph_id": "g1",
            "nodes": [
                {
                    "node_id": 5,
                    "node_name": "test_node",
                    "priority_class": "speculative",
                    "is_critical_path": False,
                    "downstream_nodes": [],
                }
            ],
        },
    )

    response = client.get("/v1/apxm/graphs/g1/priority/5")

    assert response.status_code == 200
    data = response.json()

    # Validate response structure
    assert "graph_id" in data
    assert "node_id" in data
    assert "priority" in data
    assert "priority_class" in data

    assert data["graph_id"] == "g1"
    assert data["node_id"] == 5
    assert data["priority"] == 10  # speculative
    assert data["priority_class"] == "speculative"
