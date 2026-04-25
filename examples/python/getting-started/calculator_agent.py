"""Calculator agent demo with native Python tools exposed to the LLM.

Usage:
  python3 examples/python/getting-started/calculator_agent.py
  dekk apxm execute examples/python/getting-started/calculator_agent.py
"""

from apxm import Agent, GraphRecorder, compile, run, tool


AGENT_CALCULATOR = "calculator"
PROMPT_CALCULATE = "Use the available calculator tools to compute 17 + 25."


@tool
def add(a: int, b: int) -> int:
    """Add two integers."""
    return a + b


@tool
def multiply(a: int, b: int) -> int:
    """Multiply two integers."""
    return a * b


calculator = Agent(
    name=AGENT_CALCULATOR,
    instructions="You are a calculator. Use the provided tools for arithmetic.",
    tools=[add, multiply],
)


@compile()
def math_flow(g: GraphRecorder):
    result = calculator.ask(g, PROMPT_CALCULATE)
    g.done(result)


if __name__ == "__main__":
    result = run(math_flow, mock=True)
    print(result.content)
