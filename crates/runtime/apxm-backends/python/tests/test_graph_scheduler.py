"""Tests for GraphAwareScheduler."""

import time
import pytest
from apxm_vllm.graph_scheduler import (
    GraphAwareScheduler,
    GraphMetadata,
    NodeSpec,
    PinHandle,
)


def test_scheduler_initialization():
    """Test scheduler initializes with correct defaults."""
    scheduler = GraphAwareScheduler()

    assert scheduler.default_pin_ttl_ms == 30_000
    assert len(scheduler.registered_graphs) == 0
    assert len(scheduler.active_pins) == 0
    assert scheduler.metrics["total_registrations"] == 0


def test_register_graph():
    """Test graph registration."""
    scheduler = GraphAwareScheduler()

    metadata = {
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
        "default_pin_ttl_ms": 45_000,
    }

    result = scheduler.register_graph("test-graph", metadata)

    assert result["object"] == "apxm.graph.registration"
    assert result["graph_id"] == "test-graph"
    assert result["execution_id"] == "exec-123"
    assert result["registered_nodes"] == 2

    # Check internal state
    assert "test-graph" in scheduler.registered_graphs
    graph = scheduler.registered_graphs["test-graph"]
    assert graph.node_count == 5
    assert graph.critical_path_length == 3
    assert graph.default_pin_ttl_ms == 45_000
    assert len(graph.nodes) == 2
    assert scheduler.metrics["total_registrations"] == 1


def test_get_priority_critical_path():
    """Test priority for critical path node."""
    scheduler = GraphAwareScheduler()

    metadata = {
        "nodes": [
            {
                "node_id": 1,
                "priority_class": "critical_path",
                "is_critical_path": True,
                "downstream_nodes": [2],
            }
        ]
    }

    scheduler.register_graph("g1", metadata)
    priority = scheduler.get_priority("g1", 1)

    assert priority == 0  # critical_path


def test_get_priority_parallel():
    """Test priority for parallel node."""
    scheduler = GraphAwareScheduler()

    metadata = {
        "nodes": [
            {
                "node_id": 2,
                "priority_class": "parallel",
                "is_critical_path": False,
                "downstream_nodes": [],
            }
        ]
    }

    scheduler.register_graph("g1", metadata)
    priority = scheduler.get_priority("g1", 2)

    assert priority == 5  # parallel


def test_get_priority_default():
    """Test default priority for unknown graph/node."""
    scheduler = GraphAwareScheduler()

    # Unknown graph
    priority = scheduler.get_priority("unknown-graph", 1)
    assert priority == 5  # default

    # Known graph, unknown node
    scheduler.register_graph("g1", {"nodes": []})
    priority = scheduler.get_priority("g1", 999)
    assert priority == 5  # default


def test_should_pin_prefix_explicit_policy():
    """Test pin decision with explicit policy."""
    scheduler = GraphAwareScheduler()

    metadata = {"nodes": [{"node_id": 1, "is_critical_path": True, "downstream_nodes": [2]}]}
    scheduler.register_graph("g1", metadata)

    # Explicit pin policy
    pin_policy = {"mode": "prefix", "ttl_ms": 60_000}
    should_pin, ttl = scheduler.should_pin_prefix("g1", 1, pin_policy)

    assert should_pin is True
    assert ttl == 60_000

    # Explicit no-pin policy
    pin_policy = {"mode": "none"}
    should_pin, ttl = scheduler.should_pin_prefix("g1", 1, pin_policy)

    assert should_pin is False
    assert ttl == 0


def test_should_pin_prefix_default_policy():
    """Test pin decision with default policy (critical path + downstream)."""
    scheduler = GraphAwareScheduler(default_pin_ttl_ms=30_000)

    metadata = {
        "nodes": [
            {"node_id": 1, "is_critical_path": True, "downstream_nodes": [2, 3]},
            {"node_id": 2, "is_critical_path": False, "downstream_nodes": []},
        ],
        "default_pin_ttl_ms": 30_000,
    }
    scheduler.register_graph("g1", metadata)

    # Critical path + downstream -> should pin
    should_pin, ttl = scheduler.should_pin_prefix("g1", 1, None)
    assert should_pin is True
    assert ttl == 30_000

    # Not critical path -> should not pin
    should_pin, ttl = scheduler.should_pin_prefix("g1", 2, None)
    assert should_pin is False


def test_create_pin():
    """Test creating a KV-cache pin."""
    scheduler = GraphAwareScheduler()

    pin = scheduler.create_pin(
        request_id="req-123",
        graph_id="g1",
        node_id=1,
        reuse_group="shared-prefix",
        ttl_ms=30_000,
        block_ids=[10, 11, 12],
    )

    assert pin.request_id == "req-123"
    assert pin.graph_id == "g1"
    assert pin.node_id == 1
    assert pin.reuse_group == "shared-prefix"
    assert pin.block_ids == [10, 11, 12]
    assert not pin.consumed
    assert pin.expiry_ts > time.time()

    # Check internal state
    assert ("g1", 1) in scheduler.active_pins
    assert "shared-prefix" in scheduler.pins_by_reuse_group
    assert scheduler.metrics["active_pins"] == 1
    assert scheduler.metrics["pinned_blocks"] == 3


def test_find_reusable_pin_by_reuse_group():
    """Test finding pin by reuse group (fan-out pattern)."""
    scheduler = GraphAwareScheduler()

    pin = scheduler.create_pin(
        request_id="req-1",
        graph_id="g1",
        node_id=1,
        reuse_group="analysis-context",
        ttl_ms=30_000,
        block_ids=[100, 101],
    )

    # Downstream node 2 looks for reuse_group
    found = scheduler.find_reusable_pin("g1", None, "analysis-context")

    assert found is not None
    assert found.request_id == "req-1"
    assert found.reuse_group == "analysis-context"
    assert scheduler.metrics["pin_hits"] == 1


def test_find_reusable_pin_by_upstream():
    """Test finding pin by upstream node ID (direct dependency)."""
    scheduler = GraphAwareScheduler()

    pin = scheduler.create_pin(
        request_id="req-1",
        graph_id="g1",
        node_id=1,
        reuse_group=None,
        ttl_ms=30_000,
        block_ids=[200, 201],
    )

    # Downstream node 2 depends on node 1
    found = scheduler.find_reusable_pin("g1", 1, None)

    assert found is not None
    assert found.node_id == 1
    assert scheduler.metrics["pin_hits"] == 1


def test_find_reusable_pin_miss():
    """Test pin lookup miss."""
    scheduler = GraphAwareScheduler()

    # No pins registered
    found = scheduler.find_reusable_pin("g1", 1, None)

    assert found is None
    assert scheduler.metrics["pin_misses"] == 1


def test_find_reusable_pin_expired():
    """Test that expired pins are not returned."""
    scheduler = GraphAwareScheduler()

    # Create pin with 0ms TTL (immediately expired)
    pin = scheduler.create_pin(
        request_id="req-1",
        graph_id="g1",
        node_id=1,
        reuse_group="test",
        ttl_ms=0,
        block_ids=[10],
    )

    # Should not find expired pin
    found = scheduler.find_reusable_pin("g1", None, "test")

    assert found is None
    assert scheduler.metrics["pin_misses"] == 1


def test_find_reusable_pin_consumed():
    """Test that consumed pins are not returned."""
    scheduler = GraphAwareScheduler()

    pin = scheduler.create_pin(
        request_id="req-1",
        graph_id="g1",
        node_id=1,
        reuse_group="test",
        ttl_ms=30_000,
        block_ids=[10],
    )

    # Consume the pin
    scheduler.consume_pin(pin)

    # Should not find consumed pin
    found = scheduler.find_reusable_pin("g1", None, "test")

    assert found is None
    assert scheduler.metrics["pin_misses"] == 1


def test_cleanup_expired_pins():
    """Test cleanup of expired and consumed pins."""
    scheduler = GraphAwareScheduler()

    # Create 3 pins: 1 valid, 1 expired, 1 consumed
    pin1 = scheduler.create_pin(
        "req-1", "g1", 1, "group1", 30_000, [10, 11]
    )  # valid

    pin2 = scheduler.create_pin(
        "req-2", "g1", 2, "group2", 0, [20, 21]
    )  # expired

    pin3 = scheduler.create_pin(
        "req-3", "g1", 3, "group3", 30_000, [30, 31]
    )  # consumed
    scheduler.consume_pin(pin3)

    released_blocks = scheduler.cleanup_expired_pins()

    # Should release 4 blocks (2 from expired + 2 from consumed)
    assert released_blocks == 4

    # Only pin1 should remain
    assert scheduler.metrics["active_pins"] == 1
    assert scheduler.metrics["pinned_blocks"] == 2
    assert scheduler.metrics["pin_expirations"] == 1


def test_release_graph():
    """Test graph release cleans up all pins."""
    scheduler = GraphAwareScheduler()

    metadata = {"nodes": [{"node_id": 1, "is_critical_path": True, "downstream_nodes": [2]}]}
    scheduler.register_graph("g1", metadata)

    # Create pins for g1
    scheduler.create_pin("req-1", "g1", 1, "group1", 30_000, [10, 11, 12])
    scheduler.create_pin("req-2", "g1", 2, "group2", 30_000, [20, 21])

    # Create pin for different graph
    scheduler.create_pin("req-3", "g2", 1, None, 30_000, [100])

    result = scheduler.release_graph("g1")

    assert result["object"] == "apxm.graph.release"
    assert result["graph_id"] == "g1"
    assert result["released_handles"] == 2
    assert result["released_blocks"] == 5

    # g1 should be removed
    assert "g1" not in scheduler.registered_graphs

    # Only g2 pin should remain
    assert scheduler.metrics["active_pins"] == 1
    assert scheduler.metrics["pinned_blocks"] == 1
    assert scheduler.metrics["total_releases"] == 1


def test_apply_memory_pressure_policy_low():
    """Test no action under low memory pressure."""
    scheduler = GraphAwareScheduler()

    scheduler.create_pin("req-1", "g1", 1, None, 30_000, [10, 11])

    released = scheduler.apply_memory_pressure_policy(kv_usage_pct=70.0)

    assert released == 0
    assert scheduler.metrics["active_pins"] == 1  # No change


def test_apply_memory_pressure_policy_high():
    """Test releasing non-critical pins under high pressure."""
    scheduler = GraphAwareScheduler()

    metadata = {
        "nodes": [
            {"node_id": 1, "is_critical_path": True, "downstream_nodes": []},
            {"node_id": 2, "is_critical_path": False, "downstream_nodes": []},
        ]
    }
    scheduler.register_graph("g1", metadata)

    # Create critical and non-critical pins
    scheduler.create_pin("req-1", "g1", 1, None, 30_000, [10, 11])  # critical
    scheduler.create_pin("req-2", "g1", 2, None, 30_000, [20, 21])  # non-critical

    released = scheduler.apply_memory_pressure_policy(kv_usage_pct=98.0)

    # Should release non-critical pin (2 blocks)
    assert released == 2
    assert scheduler.metrics["active_pins"] == 1
    assert scheduler.metrics["memory_pressure_releases"] == 1


def test_get_metrics():
    """Test metrics retrieval."""
    scheduler = GraphAwareScheduler()

    metrics = scheduler.get_metrics()

    assert "total_registrations" in metrics
    assert "active_pins" in metrics
    assert "pin_hits" in metrics
    assert "pin_misses" in metrics

    # Modify metrics
    scheduler.metrics["pin_hits"] = 42

    # get_metrics should return a copy
    metrics2 = scheduler.get_metrics()
    assert metrics2["pin_hits"] == 42
    metrics2["pin_hits"] = 100
    assert scheduler.metrics["pin_hits"] == 42  # Original unchanged


def test_pin_handle_is_expired():
    """Test PinHandle expiry check."""
    pin = PinHandle(
        request_id="req-1",
        graph_id="g1",
        node_id=1,
        reuse_group=None,
        expiry_ts=time.time() - 10,  # 10 seconds ago
        block_ids=[10],
    )

    assert pin.is_expired()

    pin2 = PinHandle(
        request_id="req-2",
        graph_id="g1",
        node_id=2,
        reuse_group=None,
        expiry_ts=time.time() + 3600,  # 1 hour from now
        block_ids=[20],
    )

    assert not pin2.is_expired()


def test_pin_handle_matches_reuse_group():
    """Test PinHandle reuse group matching."""
    pin = PinHandle(
        request_id="req-1",
        graph_id="g1",
        node_id=1,
        reuse_group="analysis-context",
        expiry_ts=time.time() + 3600,
        block_ids=[10],
    )

    assert pin.matches_reuse_group("analysis-context")
    assert not pin.matches_reuse_group("different-group")
    assert not pin.matches_reuse_group(None)


def test_concurrent_access():
    """Test thread-safe concurrent operations."""
    import threading

    scheduler = GraphAwareScheduler()

    def register_graphs():
        for i in range(10):
            scheduler.register_graph(f"g{i}", {"nodes": []})

    def create_pins():
        for i in range(10):
            scheduler.create_pin(f"req-{i}", "g0", i, None, 30_000, [i * 10])

    threads = [
        threading.Thread(target=register_graphs),
        threading.Thread(target=create_pins),
    ]

    for t in threads:
        t.start()
    for t in threads:
        t.join()

    # Should have registered 10 graphs and created 10 pins without race conditions
    assert scheduler.metrics["total_registrations"] == 10
    assert scheduler.metrics["active_pins"] == 10
