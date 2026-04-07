"""Graph-aware scheduler for vLLM with APXM integration.

This module implements KV-cache pinning and priority scheduling based on
APXM graph metadata. It maintains state for active graphs and provides
scheduling hints to vLLM's core scheduler.
"""

import time
from dataclasses import dataclass, field
from typing import Dict, List, Optional, Set
from threading import Lock


@dataclass
class NodeSpec:
    """Per-node specification from graph registration."""

    node_id: int
    node_name: Optional[str] = None
    estimated_prompt_tokens: Optional[int] = None
    downstream_nodes: List[int] = field(default_factory=list)
    priority_class: Optional[str] = None
    reuse_group: Optional[str] = None
    is_critical_path: bool = False


@dataclass
class GraphMetadata:
    """Full graph metadata registered with the scheduler."""

    graph_id: str
    execution_id: Optional[str] = None
    critical_path_length: Optional[int] = None
    node_count: Optional[int] = None
    max_parallelism: Optional[int] = None
    nodes: List[NodeSpec] = field(default_factory=list)
    default_pin_ttl_ms: int = 30_000  # 30 seconds default

    def __post_init__(self):
        """Build node lookup index."""
        self.node_map: Dict[int, NodeSpec] = {
            node.node_id: node for node in self.nodes
        }


@dataclass
class PinHandle:
    """Handle for a pinned KV-cache prefix."""

    request_id: str
    graph_id: str
    node_id: int
    reuse_group: Optional[str]
    expiry_ts: float  # Unix timestamp when pin expires
    block_ids: List[int] = field(default_factory=list)
    consumed: bool = False

    def is_expired(self) -> bool:
        """Check if this pin has expired."""
        return time.time() > self.expiry_ts

    def matches_reuse_group(self, group: Optional[str]) -> bool:
        """Check if this pin matches a reuse group."""
        if self.reuse_group is None or group is None:
            return False
        return self.reuse_group == group


class GraphAwareScheduler:
    """vLLM scheduler extension that uses APXM graph metadata.

    This scheduler maintains state for registered graphs and provides:
    - Priority hints based on critical path analysis
    - KV-cache pinning decisions for prefix reuse
    - Graph lifecycle management and cleanup
    - Metrics for pin effectiveness

    Thread-safe for concurrent vLLM request processing.
    """

    # Priority mapping (lower = more urgent)
    PRIORITY_MAP = {
        "critical_path": 0,
        "critical_path_non_interactive": 2,
        "normal": 5,
        "parallel": 5,
        "speculative": 10,
        "background": 15,
    }

    def __init__(self, default_pin_ttl_ms: int = 30_000):
        """Initialize the graph-aware scheduler.

        Args:
            default_pin_ttl_ms: Default TTL for KV-cache pins in milliseconds
        """
        self.default_pin_ttl_ms = default_pin_ttl_ms

        # Registered graphs indexed by graph_id
        self.registered_graphs: Dict[str, GraphMetadata] = {}

        # Active pins indexed by (graph_id, node_id) for fast lookup
        self.active_pins: Dict[tuple, List[PinHandle]] = {}

        # Pins indexed by reuse_group for shared-prefix fan-out
        self.pins_by_reuse_group: Dict[str, List[PinHandle]] = {}

        # Thread safety for concurrent access
        self._lock = Lock()

        # Metrics
        self.metrics = {
            "total_registrations": 0,
            "total_releases": 0,
            "active_pins": 0,
            "pinned_blocks": 0,
            "pin_hits": 0,
            "pin_misses": 0,
            "pin_expirations": 0,
            "memory_pressure_releases": 0,
        }

    def register_graph(self, graph_id: str, metadata: dict) -> dict:
        """Register a graph's structure for scheduling hints.

        Args:
            graph_id: Unique identifier for the graph
            metadata: Graph metadata dict matching GraphMetadata schema

        Returns:
            Registration response with node count and graph_id
        """
        with self._lock:
            # Convert nodes from dicts to NodeSpec objects
            nodes = [NodeSpec(**node) for node in metadata.get("nodes", [])]

            graph_meta = GraphMetadata(
                graph_id=graph_id,
                execution_id=metadata.get("execution_id"),
                critical_path_length=metadata.get("critical_path_length"),
                node_count=metadata.get("node_count", len(nodes)),
                max_parallelism=metadata.get("max_parallelism"),
                nodes=nodes,
                default_pin_ttl_ms=metadata.get("default_pin_ttl_ms", self.default_pin_ttl_ms),
            )

            self.registered_graphs[graph_id] = graph_meta
            self.metrics["total_registrations"] += 1

            return {
                "object": "apxm.graph.registration",
                "graph_id": graph_id,
                "execution_id": graph_meta.execution_id,
                "registered_nodes": len(nodes),
            }

    def get_priority(self, graph_id: str, node_id: int) -> int:
        """Get scheduling priority for a request based on graph position.

        Args:
            graph_id: Graph identifier
            node_id: Node identifier within the graph

        Returns:
            Priority integer (lower = more urgent), default 5 if not found
        """
        with self._lock:
            meta = self.registered_graphs.get(graph_id)
            if not meta:
                return self.PRIORITY_MAP["normal"]  # default priority

            node = meta.node_map.get(node_id)
            if not node:
                return self.PRIORITY_MAP["normal"]

            priority_class = node.priority_class or "normal"
            return self.PRIORITY_MAP.get(priority_class, self.PRIORITY_MAP["normal"])

    def should_pin_prefix(
        self,
        graph_id: str,
        node_id: int,
        pin_policy: Optional[dict] = None
    ) -> tuple[bool, int]:
        """Check if this node's KV-cache prefix should be pinned.

        Args:
            graph_id: Graph identifier
            node_id: Node identifier
            pin_policy: Optional pin policy from request hints

        Returns:
            Tuple of (should_pin: bool, ttl_ms: int)
        """
        with self._lock:
            meta = self.registered_graphs.get(graph_id)
            if not meta:
                return False, 0

            # Check explicit pin policy from request
            if pin_policy:
                mode = pin_policy.get("mode", "none")
                if mode == "none":
                    return False, 0
                ttl_ms = pin_policy.get("ttl_ms", meta.default_pin_ttl_ms)
                return mode == "prefix", ttl_ms

            # Default: pin critical path nodes with downstream dependencies
            node = meta.node_map.get(node_id)
            if node and node.is_critical_path and node.downstream_nodes:
                return True, meta.default_pin_ttl_ms

            return False, 0

    def create_pin(
        self,
        request_id: str,
        graph_id: str,
        node_id: int,
        reuse_group: Optional[str],
        ttl_ms: int,
        block_ids: List[int],
    ) -> PinHandle:
        """Create a new KV-cache pin for a completed request.

        Args:
            request_id: vLLM request identifier
            graph_id: Graph identifier
            node_id: Node identifier
            reuse_group: Optional semantic reuse group
            ttl_ms: Time-to-live in milliseconds
            block_ids: KV-cache block IDs to pin

        Returns:
            New PinHandle object
        """
        expiry_ts = time.time() + (ttl_ms / 1000.0)

        pin = PinHandle(
            request_id=request_id,
            graph_id=graph_id,
            node_id=node_id,
            reuse_group=reuse_group,
            expiry_ts=expiry_ts,
            block_ids=block_ids,
        )

        with self._lock:
            # Index by (graph_id, node_id)
            key = (graph_id, node_id)
            if key not in self.active_pins:
                self.active_pins[key] = []
            self.active_pins[key].append(pin)

            # Index by reuse_group if present
            if reuse_group:
                if reuse_group not in self.pins_by_reuse_group:
                    self.pins_by_reuse_group[reuse_group] = []
                self.pins_by_reuse_group[reuse_group].append(pin)

            # Update metrics
            self.metrics["active_pins"] += 1
            self.metrics["pinned_blocks"] += len(block_ids)

        return pin

    def find_reusable_pin(
        self,
        graph_id: str,
        upstream_node_id: Optional[int],
        reuse_group: Optional[str]
    ) -> Optional[PinHandle]:
        """Find a reusable pin for prefix KV-cache reuse.

        Args:
            graph_id: Graph identifier
            upstream_node_id: ID of upstream node that may have a pin
            reuse_group: Semantic reuse group for fan-out patterns

        Returns:
            PinHandle if found and valid, None otherwise
        """
        with self._lock:
            # First try reuse_group (for shared-prefix fan-out)
            if reuse_group and reuse_group in self.pins_by_reuse_group:
                for pin in self.pins_by_reuse_group[reuse_group]:
                    if not pin.consumed and not pin.is_expired():
                        self.metrics["pin_hits"] += 1
                        return pin

            # Fall back to direct upstream lookup
            if upstream_node_id is not None:
                key = (graph_id, upstream_node_id)
                if key in self.active_pins:
                    for pin in self.active_pins[key]:
                        if not pin.consumed and not pin.is_expired():
                            self.metrics["pin_hits"] += 1
                            return pin

            self.metrics["pin_misses"] += 1
            return None

    def consume_pin(self, pin: PinHandle):
        """Mark a pin as consumed (used for downstream request).

        Args:
            pin: PinHandle to mark as consumed
        """
        with self._lock:
            pin.consumed = True

    def cleanup_expired_pins(self) -> int:
        """Remove expired pins and return freed block count.

        Returns:
            Number of blocks released
        """
        released_blocks = 0

        with self._lock:
            # Clean active_pins
            for key in list(self.active_pins.keys()):
                pins = self.active_pins[key]
                expired = [p for p in pins if p.is_expired() or p.consumed]

                for pin in expired:
                    released_blocks += len(pin.block_ids)
                    self.metrics["active_pins"] -= 1
                    self.metrics["pinned_blocks"] -= len(pin.block_ids)
                    if pin.is_expired():
                        self.metrics["pin_expirations"] += 1

                # Keep only valid pins
                self.active_pins[key] = [p for p in pins if not (p.is_expired() or p.consumed)]
                if not self.active_pins[key]:
                    del self.active_pins[key]

            # Clean reuse_group index
            for group in list(self.pins_by_reuse_group.keys()):
                pins = self.pins_by_reuse_group[group]
                self.pins_by_reuse_group[group] = [
                    p for p in pins if not (p.is_expired() or p.consumed)
                ]
                if not self.pins_by_reuse_group[group]:
                    del self.pins_by_reuse_group[group]

        return released_blocks

    def release_graph(self, graph_id: str) -> dict:
        """Release a graph's KV-cache pins and metadata.

        Args:
            graph_id: Graph identifier to release

        Returns:
            Release response with stats
        """
        released_handles = 0
        released_blocks = 0

        with self._lock:
            # Remove all pins for this graph
            keys_to_remove = [
                key for key in self.active_pins.keys()
                if key[0] == graph_id
            ]

            for key in keys_to_remove:
                pins = self.active_pins[key]
                for pin in pins:
                    released_handles += 1
                    released_blocks += len(pin.block_ids)
                    self.metrics["active_pins"] -= 1
                    self.metrics["pinned_blocks"] -= len(pin.block_ids)
                del self.active_pins[key]

            # Clean reuse_group index
            for group in list(self.pins_by_reuse_group.keys()):
                self.pins_by_reuse_group[group] = [
                    p for p in self.pins_by_reuse_group[group]
                    if p.graph_id != graph_id
                ]
                if not self.pins_by_reuse_group[group]:
                    del self.pins_by_reuse_group[group]

            # Remove graph metadata
            if graph_id in self.registered_graphs:
                del self.registered_graphs[graph_id]

            self.metrics["total_releases"] += 1

        return {
            "object": "apxm.graph.release",
            "graph_id": graph_id,
            "released_handles": released_handles,
            "released_blocks": released_blocks,
        }

    def get_metrics(self) -> dict:
        """Get current scheduler metrics.

        Returns:
            Dict of metric name to value
        """
        with self._lock:
            return self.metrics.copy()

    def apply_memory_pressure_policy(self, kv_usage_pct: float) -> int:
        """Apply memory pressure policy and release non-critical pins if needed.

        Args:
            kv_usage_pct: Current KV-cache usage percentage (0-100)

        Returns:
            Number of blocks released
        """
        # Memory pressure thresholds from strategy doc section 6.7
        if kv_usage_pct < 85.0:
            return 0  # Allow all pins

        released_blocks = 0

        with self._lock:
            if kv_usage_pct >= 97.0:
                # Release non-critical pins immediately
                for key in list(self.active_pins.keys()):
                    graph_id, node_id = key
                    meta = self.registered_graphs.get(graph_id)

                    if not meta:
                        continue

                    node = meta.node_map.get(node_id)
                    if node and not node.is_critical_path:
                        pins = self.active_pins[key]
                        for pin in pins:
                            released_blocks += len(pin.block_ids)
                            self.metrics["active_pins"] -= 1
                            self.metrics["pinned_blocks"] -= len(pin.block_ids)
                            self.metrics["memory_pressure_releases"] += 1
                        del self.active_pins[key]

            elif kv_usage_pct >= 92.0:
                # Downgrade new pins would happen at request time
                # Here we just clean up expired pins aggressively
                released_blocks = self.cleanup_expired_pins()

        return released_blocks
