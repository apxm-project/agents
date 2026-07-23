# APXM Examples

## Why APXM?

APXM lets authors write readable typed Agents while retaining the complete
structure needed to compile, optimize, govern, execute, and explain them:

- **Simple source** — Python and TypeScript use `Agent`, `Context`, `Tool`,
  `Model`, Events, Hooks, typed composition, and ordinary control flow.
- **Compiler understanding** — one FrontendGraph captures types, values,
  dependencies, effects, Context flow, regions, loops, and source spans.
- **Safe optimization** — the Rust/AIS pipeline may optimize only when program
  meaning, authority, effect ordering, durability, and evidence correlation are
  preserved.
- **Exact execution** — Agent composition uses `new`/`invoke`; Models and
  Capabilities execute only through exact admitted ports, without hidden
  routing, fallback, or Tool loops.
- **Full observability** — source, artifact, Program Invocation, loop
  occurrence, NodeExecution, attempt, effect, Context, usage, output, and
  failure evidence remain joinable.

A conversational Agent is therefore just one repository example: an ordinary
Agent with an authored loop, explicit Context, Model/Tool calls, and
yield/resume. Events, Hooks, specialists, and structured task groups can be
added using the same generic frontend.

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

- An exact admitted inference binding for an example that executes
  `model.call` against a real model.
- Generated typed ACP profile imports for examples that invoke external coding
  agents through a Capability. Verify
  them with `dekk agents agent list` and `dekk agents agent test <name>`.
- Node/npm plus the relevant authenticated agent CLI when using APXM ACP
  profiles. The checked-in `claude` profile launches
  `npx -y @agentclientprotocol/claude-agent-acp@^0.24.2`; the checked-in
  `codex` profile launches `npx -y @zed-industries/codex-acp@^0.16.0`.
- The APXM-vLLM implementation when an exact admitted Model binding selects it;
  see `docs/backends/vllm.md`. Agent source remains provider-neutral.
- `jq` and `rg` only for inspection/reporting commands that mention
  them.

For backend-free validation, prefer compile-only checks and deterministic
contract fakes. Test fakes are explicit test dependencies, never production
fallbacks.

Canonical examples compile as source packages through the explicit bridge:

```bash
dekk agents agent build <source-package>
dekk agents compile-service-canonical <source-package> > hello.air.json
```

> **First time?** See the [docs/](../docs/README.md) overview, then run
> `dekk agents doctor` to verify your environment.

## Repository examples

The current `agents/conversational` and `agents/gao` sources are low-level
conformance scaffolds over `apxm_program` (Python) and `@apxm/frontend`
(TypeScript). They are executable baseline evidence, not the intended teaching
API. The reviewed target is the paired
[source-first guide](../docs/guides/creating-an-agent-program.md).

1. **[agents/conversational/](agents/conversational/)** -- Equivalent Python
   and TypeScript generic Agent Programs organized by an example-local
   `ConversationalAgent` helper. Validate parity with
   `dekk agents test-frontend-examples`.
2. **[agents/gao/](agents/gao/)** -- TypeScript-only Agent Program example
   that specializes the example-local helper with Capability calls, specialist
   composition, static Hooks, and `await.event`. Gao has no package, compiler,
   runtime, Server, or Studio meaning.
3. **[agents/coder/](agents/coder/)** and
   **[agents/coder-fixture/](agents/coder-fixture/)** -- ACP coding-agent
   integration examples.
4. **[workflows/](workflows/)** -- Native `.apxmw` workflow coordination,
   event loops, resume, and cancel.
5. **[metrics/](metrics/)** -- APXM graph metrics hierarchy reference.

A retired raw-authoring example corpus (`python/`, `agents/explorer/`) that
imported removed `apxm` package symbols (`GraphRecorder`, `GraphBuilder`,
`compile`, `Agent`) has been removed. It predated the current
`apxm_program` conformance scaffold and could not run.
