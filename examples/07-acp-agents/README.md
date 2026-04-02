# ACP Multi-Agent Examples

Example graphs demonstrating multi-agent workflows using ACP (Agent Communication Protocol).

## Prerequisites

Verify that the agents you want to use are reachable:

```bash
apxm agent test claude
apxm agent test codex
```

## Running

```bash
apxm validate <file>.json
apxm execute <file>.json
```

## Examples

### Style A: SPAWN_AGENT + COMMUNICATE (explicit lifecycle)

| File | Description |
|------|-------------|
| `spawn-communicate-basic.json` | Spawn a single Claude agent in architect mode and send one message |
| `parallel-agents.json` | Spawn Claude and Codex in parallel, send the same prompt, merge results |
| `multi-turn-communicate.json` | Multi-turn conversation: analyze, fix, verify with sequential COMMUNICATE nodes |
| `cross-critique.json` | Two agents independently propose, then critique each other's proposals |
| `full-sdlc.json` | Pipeline: architect designs, coder implements, architect reviews |

### Style B: INV with ACP capability (session-managed)

| File | Description |
|------|-------------|
| `parallel-review.json` | Ask Codex and Claude the same question in parallel, then compare answers |
| `multi-turn-review.json` | Multi-turn session (analyze, fix, verify) using a shared session handle |

## Further Reading

- [Multi-Agent Guide](../../docs/guides/multi-agent.md) -- full walkthrough of concepts, setup, and patterns
- [Common Patterns](../../docs/guides/common-patterns.md) -- single-agent graph patterns
