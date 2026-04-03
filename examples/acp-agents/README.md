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
apxm validate <file>.apxm
apxm execute <file>.apxm
```

## Examples

### Style A: SPAWN_AGENT + COMMUNICATE (explicit lifecycle)

| File | Description |
|------|-------------|
| `spawn-communicate-basic.apxm` | Spawn a single Claude agent in architect mode and send one message |
| `parallel-agents.apxm` | Spawn Claude and Codex in parallel, send the same prompt, merge results |
| `multi-turn-communicate.apxm` | Multi-turn conversation: analyze, fix, verify with sequential COMMUNICATE nodes |
| `cross-critique.apxm` | Two agents independently propose, then critique each other's proposals |
| `full-sdlc.apxm` | Pipeline: architect designs, coder implements, architect reviews |

### Style B: INV with ACP capability (session-managed)

| File | Description |
|------|-------------|
| `parallel-review.apxm` | Ask Codex and Claude the same question in parallel, then compare answers |
| `multi-turn-review.apxm` | Multi-turn session (analyze, fix, verify) using a shared session handle |

## Further Reading

- [Multi-Agent Guide](../../docs/guides/multi-agent.md) -- full walkthrough of concepts, setup, and patterns
- Run `apxm template list` to browse single-agent graph patterns
