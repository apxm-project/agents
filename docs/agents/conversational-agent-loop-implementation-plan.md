# Conversational Agent loop implementation plan — superseded

- Status: superseded on 2026-07-16
- Replacement plan: [Agent Program composition and AIR full-replacement plan](agent-program-composition-and-air-full-replacement-plan.md)
- Replacement contract: [Agent Program composition and AIR contract](agent-program-composition-and-air-contract.md)

The old L1-L7 plan would have implemented a runtime Turn lifecycle. The target
instead compiles a program-authored loop to generic structured AIR and projects
runtime region occurrences as Turns in Studio. No target implementation may
use this historical plan.
