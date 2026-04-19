"""Calculator Agent demo — validates that Agent.ask() lowers to correct AIR ops.

This is the MVP "killer demo" from the native_python_tools.md design doc.
Usage: dekk apxm compile examples/python/getting-started/calculator_agent.py
"""

from apxm import tool, Agent, GraphRecorder
from apxm._generated.models import Anthropic


@tool
def add(a: int, b: int) -> int:
    """Add two integers."""
    return a + b


@tool
def multiply(a: int, b: int) -> int:
    """Multiply two integers."""
    return a * b


calc = Agent(
    name="calc",
    instructions="You are a calculator. Use the provided tools to compute answers.",
    tools=[add, multiply],
    model=Anthropic.CLAUDE_SONNET_4_6,
)

g = GraphRecorder("math_flow")
g.param("q", "str")

result = calc.ask(g, "{q}")
g.done(result)

print(g.to_air())
