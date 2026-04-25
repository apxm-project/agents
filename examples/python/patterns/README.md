# Patterns

Reusable workflow patterns for common agent tasks.

## Why This Matters

These patterns solve recurring problems: iterative refinement, cross-critique,
and resilient delegation. They can be composed into larger workflows.

## Examples

- **iterative_refine.py** -- Generate, critique, and refine in a loop-free chain. `python examples/python/patterns/iterative_refine.py`
- **cross_critique.py** -- Two agents critique each other's work. `python examples/python/patterns/cross_critique.py`
- **resilient_pipeline.py** -- Spawn worker, send task, handle result with retry semantics. `python examples/python/patterns/resilient_pipeline.py`
- **subflow_policy_call.py** -- Attach a stricter policy to the `FLOW_CALL` node that invokes a subflow. `python examples/python/patterns/subflow_policy_call.py`
- **workflow_spawn.py** -- Run a child graph as a separate execution with its own session root. `python examples/python/patterns/workflow_spawn.py`
- **local_runtime_controls.py** -- Run through the local CLI with hooks, built-in middleware, and an explicit session root. `python examples/python/patterns/local_runtime_controls.py`
- **e2e_runtime_superpowers.py** -- Self-verifying end-to-end run that checks local parent sessions, explicit child sessions, hook output, middleware config, and workflow spawn together. `PYTHONPATH=crates/compiler/apxm-frontend/python APXM_MOCK_BACKEND=1 python examples/python/patterns/e2e_runtime_superpowers.py`

## Key API

```python
from apxm import NodePolicy, WorkflowTargetKind
from apxm._generated.agents import claude

# Iterative refinement: generate -> critique -> refine
draft = g.ask(name="draft", prompt="Write a proposal")
critique = g.think(name="critique", prompt="Find weaknesses in: {draft}")
final = g.ask(name="final", prompt="Improve based on feedback: {draft}\n{critique}")

# Resilient delegation
worker = g.spawn("worker", profile=claude, cwd=cwd)
result = worker.ask("{formatted}")

# Tighten the call-site node relative to the graph default
research = g.call(
    helper_flow,
    topic=topic,
    node_policy=NodePolicy(tool_groups=["file:read"], token_budget=128),
)

# Cross the execution boundary instead of inlining/calling a registered flow
child = g.workflow_spawn(
    target_kind=WorkflowTargetKind.GRAPH_PATH,
    target="tests/quality_fixtures/qa_factual/graph.air",
    session_root=".apxm/child-sessions",
)
```

## Learn More

- [multi-agent/](../multi-agent/) -- More coordination patterns
- [real-world/](../real-world/) -- Full production workflows using these patterns
