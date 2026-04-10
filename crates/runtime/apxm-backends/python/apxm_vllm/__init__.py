"""APXM vLLM Graph-Aware Scheduler Extension.

This module extends vLLM with APXM graph metadata awareness, enabling:
- Priority-aware request scheduling based on critical path analysis
- KV-cache pinning hints for prefix reuse across graph nodes
- Graph-level lifecycle management and cleanup
- Metrics for cache hit rates and pin effectiveness

Integration:
    from apxm_vllm import GraphAwareScheduler

    scheduler = GraphAwareScheduler()
    scheduler.register_graph(graph_id="dag-123", metadata={...})
    priority = scheduler.get_priority("dag-123", node_id=5)
"""

from .graph_scheduler import GraphAwareScheduler, PinHandle

__version__ = "0.1.0"
__all__ = ["GraphAwareScheduler", "PinHandle"]
