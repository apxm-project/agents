# Start authoring an Agent

- Audience: Python and TypeScript Agent Program authors
- Frontend status: source-first authoring is the public frontend direction;
  compile the checked-in references to confirm the checkout you are using
- Authority: [ADR-0015](../adr/0015-source-first-agent-frontend-vocabulary.md),
  [ADR-0016](../adr/0016-tool-authoring-and-handler-execution-are-separate.md),
  [ADR-0013](../adr/0013-core-semantics-are-closed-and-implementations-enter-through-exact-port-bindings.md),
  and [ADR-0022](../adr/0022-capability-references-resolve-against-a-catalogue-and-permissions-are-declared-requests.md),
  which amends the first two

## Start with one ordinary Agent

Start with the [Conversational reference](../../examples/agents/conversational/README.md).
It is the primary, cross-language example: a typed `Agent` reads explicit
`Context`, calls a `Model` and a `Tool`, composes a short-lived specialist, and
uses an ordinary source loop with `agent.yield_()`.

That shape is intentionally not a framework within the framework. APXM has no
package-level `ConversationalAgent`, conversational runtime, or core `Turn`
type. It is one generic Agent Program whose source makes the behavior visible
the generic Agent Program contract.

Use the smallest vocabulary that describes your program:

```text
Agent     Context     Tool     Model     ordinary language control flow
```

`Capability`, `Event`, `Hook`, and `TaskGroup` are focused additions when the
program actually needs them. Do not start from graph ids, AIR, `ais.*` names,
runtime objects, endpoints, credentials, grants, or a hidden model/tool loop;
those are compiler, runtime, or admission concerns rather than authored
behavior ([ADR-0015](../adr/0015-source-first-agent-frontend-vocabulary.md)).

## Use the focused extensions as patterns

The reference stays the teaching path. Read the focused examples only when
their behavior matches the program you are writing:

| Need | Read | Take from it |
| --- | --- | --- |
| A coding-oriented review/proposal flow | [Coder](../../examples/agents/coder/README.md) | Typed read, edit-proposal, and test-command capabilities; the program proposes work and never mutates files or runs commands itself. |
| APXM authoring | Ordinary Agent Program source | Explicit Context, Model calls, Capabilities, and yield/resume. |

Coder and the conversational example build on the same installed generic
frontend. They are not imports, base classes, runtime modes, or privileged
identities. Copy the small generic behavior pattern you need into a new
ordinary Agent; do not make your program depend on an example name.

## A professional authoring loop

1. Define typed input, output, and only the Context that must survive a
   stateful invocation.
2. Bind exact Models and Tool/Capability references at module scope, importing
   builtin capability ids from `apxm_program.capabilities` or
   `@apxm/frontend/capabilities` rather than retyping them. Those module-scope
   declarations are the Agent's declarations — nothing lists them a second time.
   Make every effect call and every Context replacement explicit in the Agent
   body.
3. Use normal `if`, `match`/`switch`, loops, `try`/`catch`, and `return` for
   behavior. Use `agent.yield_(output)` only when the Program Instance should
   commit its next Context and resume on its next input; use an `Event` wait to
   park the same invocation instead. Use `agent.ask_owner(...)` when the
   program needs its human owner to decide before it continues: it is a yield
   whose committed output is a typed request (a prompt, literal `choices` or a
   typed `answer`, and `expires_in_seconds`) and whose resume input is the
   closed envelope `{outcome: "answered", answer} | {outcome: "declined"} |
   {outcome: "expired"}`, validated against the request before the program
   resumes. The host records who owns the Session and lets only that Person
   answer; an answer never dispatches a Capability, so a Capability the program
   calls afterwards is still governed by its own Policy.
4. If a package supplies a local Tool implementation, keep it in a handler
   module — `Tool.define` from `@apxm/agent-packaging` in TypeScript,
   `capability(...)` from `apxm_program.handlers` in Python — never in the Agent
   Program source. Both forms become executable: `HandlerLanguage` admits
   `python` and `typescript`, and a package that ships
   `capabilities/<id>/handler.py` or `handler.ts` is discovered, bundled,
   registered, and executed through the same chokepoint
   ([ADR-0016](../adr/0016-tool-authoring-and-handler-execution-are-separate.md),
   as amended by [ADR-0022](../adr/0022-capability-references-resolve-against-a-catalogue-and-permissions-are-declared-requests.md)).
   Only discovery, bundling, and the worker process differ by language.
   The generated manifest is build output; it is not source to edit and it does
   not make Node the Agent runtime or grant authority.
   If the package needs to narrow what its program asked for, state it in
   `agent.toml [permissions]` — the one authored layer above the program's
   request, and one that may only tighten
   ([the package format](agent-package-format.md#5-permissions-the-tighten-only-layer)).
5. Compile and validate references through the repository surface:

   ```sh
   dekk agents test-frontend-examples
   ```

Before opening a change that affects the authoring contract, run the static
and generated-doc gates as well. `check` is the aggregate metadata/parity/
surface gate; `check-deversion` catches a reintroduced Agents-owned versioned
identifier or filename; and `check-agent-skills` validates the discovery roots
that the runtime serves.

```sh
dekk agents check
dekk agents check-deversion
dekk agents check-agent-skills
```

For a complete local verification pass, add `dekk agents build`, `dekk agents test`,
and `dekk agents test-all`, then run the focused shipping-path checks listed in
the [verification runbook](../README.md#verification).

The frontend captures source intent; Rust selects and verifies AIR/AIS; an
admitted runtime executes the artifact. This keeps source readable without
turning Python or TypeScript into a second runtime
([ADR-0015](../adr/0015-source-first-agent-frontend-vocabulary.md)).

## Continue from here

- [Author an Agent](creating-an-agent-program.md) for Python/TypeScript
  declarations, composition, Context, and Tool details.
- [Build a conversational Agent](creating-a-conversational-agent.md) for the
  complete stateful loop, Model/Tool dispatch, events, and Hooks.
- [Compose Agent Programs](composing-agent-programs.md) when one Agent needs a
  one-shot or stateful specialist.
