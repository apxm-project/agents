# Hook and Context implementation plan — superseded

- Status: superseded on 2026-07-16
- Replacement plan: [Agent Program composition and AIR full-replacement plan](agent-program-composition-and-air-full-replacement-plan.md)
- Replacement contract: [Agent Program composition and AIR contract](agent-program-composition-and-air-contract.md)

The old H0-H8 plan implemented `HookContext`, `HookResult`, Context
Delta/Merge, and a separate conversational lifecycle. Those targets were
superseded and must not be implemented. The replacement plan owns Agent Facade
callbacks, explicit context/state, program composition, minimal AIR, Gao, and
legacy eradication together.
