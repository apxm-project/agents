#!/usr/bin/env python3
"""Local CLI-backed hooks, middleware, and explicit session root."""

from apxm import (
    ExecutionOptions,
    GraphRecorder,
    HookConfig,
    HookEvent,
    LoopGuardMiddlewareConfig,
    TimeoutMiddlewareConfig,
    compile,
    local_apxm_path,
)


@compile()
def hello(g: GraphRecorder):
    result = g.ask(name="hello", prompt="Say hello in one sentence.")
    g.done(result)


if __name__ == "__main__":
    hook_log = local_apxm_path("hook.log")
    options = ExecutionOptions(
        session_root=local_apxm_path("sessions"),
        hooks=[
            HookConfig(
                event=HookEvent.NODE_COMPLETE,
                command=f"printf %s {{{{node_id}}}} >> {hook_log}",
            )
        ],
        middlewares=[
            TimeoutMiddlewareConfig(default_timeout_ms=5_000),
            LoopGuardMiddlewareConfig(max_repeats=2),
        ],
    )
    result = hello.run_sync(execution=options)
    print(result.content)
