"""Calculator Agent demo — end-to-end execution with native Python tools.

This is the MVP "killer demo" from the native_python_tools.md design doc.

Usage:
  APXM_MOCK_BACKEND=1 python examples/python/getting-started/calculator_agent.py
  dekk apxm execute examples/python/getting-started/calculator_agent.py
"""

from apxm import tool, Agent, compile, run
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


@compile()
def math_flow(g, q: str):
    result = calc.ask(g, "{q}")
    g.done(result)


if __name__ == "__main__":
    result = run(math_flow, "What is 17 + 25?", mock=True)
    print(result.content)
