# Tool authoring and handler-execution delivery plan

Status: active implementation plan

Authority: [ADR-0016](../adr/0016-tool-authoring-and-handler-execution-are-separate.md),
[ADR-0015](../adr/0015-source-first-agent-frontend-vocabulary.md), the
[Agent Program composition and AIR contract](agent-program-composition-and-air-contract.md),
and the [compiler bridge plan](compiler-bridge-delivery-plan.md).

## Outcome

An Agent author writes typed Python or TypeScript source for a `Tool` reference
and receives a typed result value. A TypeScript package-handler author writes
one `Tool.define` object and returns `Tool.answer({...})`. Neither author
handles JSON Schema, worker frames, handler IDs, grants, or runtime selection.

Rust owns the serialized handler contract and execution boundary. A private
language worker is only an injected implementation detail behind the admitted
Capability port. There is no frontend runtime, Node default, Python default,
or language-selected fallback.

## Invariants

1. Python and TypeScript Tool references record equivalent typed
   FrontendGraph Tool intents and lower only to Rust-selected
   `capability.invoke`.
2. `apxm.handler-manifest.v1` is the only serialized handler sidecar. It is
   generated and Rust-validated; source authors never edit it.
3. `Tool.define`, `Tool.object`, `Tool.text`, and `Tool.answer` are the only
   TypeScript package-handler definition/result forms. The legacy raw-schema
   helper has no compatibility alias.
4. A handler result is an ordinary typed value after the private worker
   boundary. No answer envelope leaks into Agent Program source or a Model Tool
   result.
5. Capability ID, read-only classification, approval, grant, and exact
   implementation binding are owned outside the handler source.
6. Python package-local handlers remain unsupported until their deterministic
   bundler and admitted worker adapter prove conformance to the same Rust
   manifest. Python Tool *references* remain fully supported now.

## Delivery sequence

### T1 — Freeze the boundary and teaching surface

- Publish ADR-0016 and link it from the Agents ADR index.
- Teach the distinction between an Agent Program Tool reference, a private
  package-handler definition, the generated manifest, and Rust execution.
- Remove current guidance that presents Coder or Gao as a Studio feature or a
  TypeScript runtime.

Completion: current examples and guides name one owner for each concern and
contain no raw handler protocol instructions.

### T2 — Complete the TypeScript package-handler object

- Replace raw-schema handler declarations with `Tool.define`.
- Generate the existing manifest schema from `Tool.object` and `Tool.text`.
- Require `Tool.answer` and unwrap it only in the private packaging worker.
- Add positive and rejection tests: non-object input, raw/plain handler return,
  generated schema, and no envelope leakage.

Completion: Coder and Gao package-handler sources contain no handwritten JSON
Schema or protocol frame; regenerated sidecars validate through the Rust CLI.

### T3 — Prove Rust runtime isolation

- Keep `HandlerManifest` and all artifact validation in Rust.
- Add or retain a runtime conformance test proving `CapabilityPort` receives an
  ordinary typed result and has no Node/Python source-language branch.
- Treat the Node worker only as a bundling conformance fixture. It must not be
  included in a runtime profile or selected at runtime by name.

Completion: Rust runtime tests demonstrate source-language-independent
Capability execution; packaging tests demonstrate only bundle correctness.

### T4 — Keep frontend references equivalent

- Run the Python and TypeScript frontend suites plus source-surface alignment.
- Add matching Tool-reference fixtures when either frontend changes its public
  Tool declaration behavior.
- Do not add a Python handler API until its bundler, worker adapter, manifest
  vectors, and Rust runtime conformance land as one owner change.

Completion: frontends remain source-first, statically captured, and equivalent
without pretending that package-handler execution exists in both languages.

### T5 — Finish the examples and artifact evidence

- Keep only Conversational, Coder, and Gao under `examples/agents/`.
- Regenerate Coder/Gao manifests and the source-derived runtime-proof fixtures
  from source. Fixtures live with Rust conformance tests, never in an agent
  package, so example folders contain author-owned inputs and generated package
  sidecars only.
- Run the example, canonical-only, frontend-surface, handler, and owner check
  gates before release of the change.

## Verification

Run the smallest applicable owner gates first, then the aggregate check:

```sh
dekk agents agent sync examples/agents/coder
dekk agents agent sync examples/agents/gao
dekk agents agent build examples/agents/coder
dekk agents agent build examples/agents/gao
dekk agents test-frontend-examples
dekk agents test-typescript-frontend
dekk agents test-python-frontend
dekk agents test-cli
dekk agents check-frontend-surface
dekk agents check-example-artifacts
dekk agents test-canonical-only
dekk agents check
```

The Node worker may run only as a package-build conformance test. It is never a
runtime verification substitute.
