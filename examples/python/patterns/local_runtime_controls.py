#!/usr/bin/env python3
"""Local CLI-backed hooks, middleware, and explicit session root."""

from pathlib import Path

from apxm import (
    ExecutionOptions,
    GraphRecorder,
    HookConfig,
    HookEvent,
    LoopGuardMiddlewareConfig,
    TimeoutMiddlewareConfig,
    compile,
)


@compile()
def hello(g: GraphRecorder):
    result = g.ask(name="hello", prompt="Say hello in one sentence.")
    g.done(result)


if __name__ == "__main__":
    options = ExecutionOptions(
        session_root=Path(".apxm/sessions"),
        hooks=[
            HookConfig(
                event=HookEvent.NODE_COMPLETE,
                command="printf %s {{node_id}} >> .apxm/hook.log",
            )
        ],
        middlewares=[
            TimeoutMiddlewareConfig(default_timeout_ms=5_000),
            LoopGuardMiddlewareConfig(max_repeats=2),
        ],
    )
    result = hello.run_sync(execution=options)
    print(result.content)
