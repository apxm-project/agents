# Gao extension

Gao is the APXM-capability extension of the primary
[Conversational example](../conversational/README.md). It is a TypeScript
`Agent` that uses the public APXM frontend and exactly three APXM-specific
capabilities:

1. `capability_discovery` obtains available capability metadata;
2. `plan_workflow` turns a request into a reviewable workflow plan; and
3. `prepare_validation` prepares that plan for the owning validation surface.

Gao keeps the conversational shape—explicit Context, authored loop, Model
reply, and yield/resume—but it neither writes workflows nor runs them. Those
actions need a separately admitted host capability and explicit authority.
Gao has no package export, compiler branch, runtime mode, Server route, or
privileged identity.

The two package-local handlers use `@apxm/agent-packaging`; they are ordinary
capability implementations, not frontend or Agent Program APIs. `Tool.define`
declares each handler, `Tool.object`/`Tool.text` define its typed input, and
`Tool.answer` returns one typed plan or validation-request object. The generated
handler manifest is internal build output, so authors never hand-write protocol
frames or JSON schema.

The TypeScript helper only builds the Rust-owned handler manifest. It is not a
runtime or an authority path; the admitted Rust Capability port owns execution.

Run the focused build, compile, and authoring checks from the repository root:

```sh
dekk agents test-frontend-examples
```

`package.json` declares the example's build, compile, run, and test entrypoints
consumed by the repository control plane.
