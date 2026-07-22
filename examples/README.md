# APXM Examples

## Why APXM?

APXM compiles agent workflows into optimized execution plans. Instead of writing
imperative coordination code, you declare *what* your agents should do and the
compiler figures out *how* to run it efficiently:

- **Implicit parallelism** -- independent nodes run concurrently without manual threading
- **Compiler optimizations** -- prompt construction, dead context elimination, template specialization, scheduling hints, and execution metrics
- **Multi-agent coordination** -- native spawn, communicate, and team primitives
- **Multi-provider routing** -- assign the right model to each task (fast, powerful, local)

## Quick Start

```bash
# From project root
dekk agents doctor
dekk agents agent build <source-package>
dekk agents compile-service-canonical <source-package> > /tmp/apxm-tool-use.air.json
```

Use Dekk for normal runs. It sets up the APXM environment consistently across
machines.

## Dependencies

Required:

- `dekk agents install --no-interactive`
- `dekk agents doctor`

Optional, depending on the example:

- A registered LLM backend for examples that execute `ask`, `think`, or
  `reason` nodes against a real model.
- Generated typed ACP profile imports for examples that spawn coding agents. Verify
  them with `dekk agents agent list` and `dekk agents agent test <name>`.
- Node/npm plus the relevant authenticated agent CLI when using APXM ACP
  profiles. The checked-in `claude` profile launches
  `npx -y @agentclientprotocol/claude-agent-acp@^0.24.2`; the checked-in
  `codex` profile launches `npx -y @zed-industries/codex-acp@^0.16.0`.
- The APXM vLLM fork for `self-hosted/vllm_graph_smoke.py`; see
  `docs/backends/vllm.md`.
- `jq` and `rg` only for inspection/reporting commands that mention
  them.

For backend-free validation, prefer compile-only checks or examples that set
`mock=True` in Python. Do not bake a model id into a public example; register a
backend with `dekk agents backend ...` and select it from the APXM backend
registry.

Canonical examples compile as source packages through the explicit bridge:

```bash
dekk agents agent build <source-package>
dekk agents compile-service-canonical <source-package> > hello.air.json
```

> **First time?** See the [docs/](../docs/README.md) overview, then run
> `dekk agents doctor` to verify your environment.

## Repository examples

These author against the canonical `apxm_program` (Python) / `@apxm/frontend`
(TypeScript) Agent Program APIs — `AgentProgram`, `ConversationalAgent`,
Hooks, and Context:

1. **[agents/conversational/](agents/conversational/)** -- Equivalent Python
   and TypeScript `ConversationalAgent` programs over the installed generic
   frontends; the parity high-water mark. Validate with
   `dekk agents test-frontend-examples`.
2. **[agents/gao/](agents/gao/)** -- TypeScript-only Agent Program example
   that specializes `ConversationalAgent` with Capability calls, specialist
   composition, static Hooks, and `await.event`.
3. **[agents/coder/](agents/coder/)** and
   **[agents/coder-fixture/](agents/coder-fixture/)** -- ACP coding-agent
   integration examples.
4. **[workflows/](workflows/)** -- Native `.apxmw` workflow coordination,
   event loops, resume, and cancel.
5. **[metrics/](metrics/)** -- APXM graph metrics hierarchy reference.

A retired raw-authoring example corpus (`python/`, `agents/explorer/`) that
imported removed `apxm` package symbols (`GraphRecorder`, `GraphBuilder`,
`compile`, `Agent`) has been removed. It predated the canonical
`apxm_program` API and could not run.
