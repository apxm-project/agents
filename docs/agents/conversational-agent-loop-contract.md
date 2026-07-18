# Conversational Agent loop contract — superseded

- Status: superseded on 2026-07-16
- Superseded decision: [ADR-0005](../adr/0005-conversational-agent-lowers-to-a-structured-ais-loop.md)
- Replacement decision: [ADR-0010](../adr/0010-agent-program-source-owns-context-hooks-and-conversational-loops.md)
- Canonical contract: [Agent Program composition and AIR contract](agent-program-composition-and-air-contract.md)

The former runtime-facing `apxm.conversational-loop.v1`, Turn Outcome, terminal
Turn commit, and rearm contract is not part of the target. A Conversational
Agent authors an ordinary structured loop. The compiler preserves source
mapping, the runtime records generic region occurrences, and Studio projects
them as Turns.

This tombstone preserves link history only. It is not legacy support.
