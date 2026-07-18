# Hook and Context contract — superseded

- Status: superseded on 2026-07-16
- Superseded decision: [ADR-0001](../adr/0001-hooks-are-typed-callbacks-over-scoped-context.md)
- Replacement decision: [ADR-0010](../adr/0010-agent-program-source-owns-context-hooks-and-conversational-loops.md)
- Canonical contract: [Agent Program composition and AIR contract](agent-program-composition-and-air-contract.md)

The former `HookContext`/`HookResult` and Context Delta/Merge contract is not a
target or compatibility contract. Hooks now receive the portable Agent Facade,
context is explicit program-local/state data, and old artifacts are rejected.

This tombstone exists only so historical links fail toward the replacement
contract rather than silently describing an executable legacy path.
