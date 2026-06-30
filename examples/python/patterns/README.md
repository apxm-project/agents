# Patterns

Reusable workflow patterns for common agent tasks.

## Why This Matters

These patterns solve recurring problems: iterative refinement, cross-critique,
and resilient delegation. They can be composed into larger workflows.

## Requirements

Patterns that spawn agents use APXM ACP profile imports. Compile-only checks work
without live agents, but execution requires the profile command and
CLI/auth setup to pass `dekk agents agent test <name>`. The checked-in
`claude` profile runs
`npx -y @agentclientprotocol/claude-agent-acp@^0.24.2` and needs Claude Code
configured locally.

## Examples

- **iterative_refine.py** -- Generate, critique, and refine in a loop-free chain. `dekk agents execute examples/python/patterns/iterative_refine.py`
- **cross_critique.py** -- Two agents critique each other's work. `dekk agents execute examples/python/patterns/cross_critique.py`
- **resilient_pipeline.py** -- Spawn worker, send task, handle result with retry semantics. `dekk agents execute examples/python/patterns/resilient_pipeline.py`
- **subflow_policy_call.py** -- Attach a stricter policy to the `FLOW_CALL` node that invokes a subflow. `dekk agents execute examples/python/patterns/subflow_policy_call.py`
- **workflow_spawn.py** -- Run a child graph as a separate execution with its own session root. `dekk agents execute examples/python/patterns/workflow_spawn.py`
- **local_runtime_controls.py** -- In-process runtime-control smoke test with hooks, middleware, and an explicit session root. `python3 examples/python/patterns/local_runtime_controls.py`
- **runtime_controls_e2e.py** -- In-process end-to-end check for parent sessions, explicit child sessions, hook output, middleware config, and workflow spawn. `python3 examples/python/patterns/runtime_controls_e2e.py`

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
    target_kind=WorkflowTargetKind.AIR_PATH,
    target="workflows/review.air",
    session_root=".apxm/child-sessions",
)
```

## Learn More

- [multi-agent/](../multi-agent/) -- More coordination patterns
- [real-world/](../real-world/) -- Full production workflows using these patterns
