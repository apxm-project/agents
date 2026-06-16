# Quickstart: Agent in One Program

How to build, run, and validate the feature. Run guide, not implementation.

## Author the agent
Write one program with `ConversationalAgent(...)` (see
`contracts/python-api.md`); the acceptance fixture is
`examples/python/conversational/controllable_agent.py`.

## Validate the artifact without a server
```
PYTHONPATH=crates/compiler/apxm-frontend/python \
  python3 examples/python/conversational/controllable_agent.py --validate
```
Expect: VALID; one multi-flow artifact; hooks + sub-agent flows present.

## Compile to an artifact
```
dekk apxm execute examples/python/conversational/controllable_agent.py --emit-air > agent.air
```

## Run on host A — interactive terminal
```
apxm chat --air agent.air --server http://127.0.0.1:18800 \
  --import support --import engineering --import docs --import analysis
```
The host should only pipe stdin and render tokens; all behavior comes from the
program.

## Run on host B — hosted service (dumb pipe)
1. Start the agent once: `POST /v1/execute/stream` with `agent.air` + a
   `session_id` + `imports` for the visible skill libraries; keep the stream
   open.
2. Send turns: `POST /v1/conversations/{session_id}/message` with each user line.
3. Render the streamed reply.

## Acceptance validation (maps to spec Success Criteria)
- **SC-001:** the fixture holds a multi-turn conversation using a tool and
  recalling turn 1 — no host logic.
- **SC-002:** run the same `agent.air` on host A and host B; replies equivalent;
  every hook fires on both.
- **SC-003:** drive ≥50 turns; ask about a turn-1 fact; correct answer; stays
  within the working limit.
- **SC-004:** observe the block hook stop a tool call and the edit hook change
  one, on both hosts.
- **SC-005:** issue 10 representative requests; ≥8 select the right skill by
  description.
- **SC-006:** a delegated sub-agent's result appears in the reply.
- **SC-007:** sending one more message does not recompute prior turns.

## Tests to keep green
`cargo test` for runtime / server / compiler (park/wake, op-invariants, security
suites); Python `--validate`.

## Skill catalogue for SC-005

Use pack layout for imported libraries:
`<skill-root>/<pack-id>/pack.toml` plus
`<skill-root>/<pack-id>/skills/<skill-id>/skill.toml`. The `pack.toml`
`pack_id` is what `--import <pack-id>` / request `imports` makes visible to
`search_skills`; `shared = true` skills remain globally visible.
