# Bug: spawn_agent incompatible with flow parameters — opaque E900 error

**Status:** Open  
**Severity:** High — causes silent wrong behavior in workflows  
**Filed:** 2026-04-06  

## Repro

```ais
agent Test {
    @entry flow main(TASK: str) -> str {
        spawn_agent("coder", "claude", "$APXM_HOME") -> _coder
        think("task: {{TASK}}") -> result
        communicate("coder", "acp", result) -> out
        return out
    }
}
```

```
dekk apxm compile bug_repro.ais
→ error: expected ':'
     --> "-":3:34
  Error: Internal compiler error: error[E900]: module parsing: operation returned null
```

## Expected behavior

A clear, actionable compiler error:

```
error[E042]: spawn_agent is not supported in flows with parameters
  --> bug_repro.ais:3:9
  |
3 |         spawn_agent("coder", "claude", "...") -> _coder
  |         ^^^^^^^^^^^ cannot use spawn_agent when entry flow has parameters
  |
  = note: spawn_agent requires @entry flow main() with no parameters
  = help: move task input to ask() or const_str() inside the flow body
```

## Actual behavior

Opaque internal error E900 with no indication of what is wrong or how to fix it.

## Silent failure path (the real danger)

When the workflow is restructured to work around the bug using `ask()` to receive
the task instead of a flow parameter:

1. Workflow compiles successfully ✅
2. `ask()` sends "What coding task should ultrathink implement?" to the LLM
3. LLM halluminates a task description
4. Workflow runs 5+ minutes, spends thousands of tokens, Claude Code commits code
5. All of it solves the **wrong problem** — zero indication anything went wrong

This happened in practice: the Claude Code agent committed
`feat(runtime): add task context validation for model router` which had to be reverted.

## Root cause (hypothesis)

The AIS compiler's MLIR lowering for `spawn_agent` does not handle the case where
the enclosing flow has a non-empty parameter list. The parser sees the flow params
then fails to parse `spawn_agent` correctly, producing a null operation.

## Workaround (current)

Use `ask()` inside a no-param `main()`. **This is semantically wrong** — `ask()`
queries the LLM, not the caller. Task comes from the model's imagination.

## Fix needed

1. Emit a clear diagnostic when `spawn_agent` is used in a parameterized flow
2. Fix the underlying MLIR lowering so `spawn_agent` works in parameterized flows

## Impact

Any workflow needing to accept a task string AND spawn agents is broken.
This includes the ultrathink-coder workflow.
