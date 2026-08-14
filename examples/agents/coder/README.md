# Coder extension

Coder is the focused coding-capability extension of the primary
[Conversational example](../conversational/README.md). It is an ordinary
source-first TypeScript `Agent` built on the public APXM frontend.

The entire flow is deliberately small:

1. `read` inspects one requested file;
2. `edit` returns a structured before/after proposal; and
3. `test` returns a single test-command proposal.

Coder never writes a file or executes a command: both package-local handlers
return a proposal marked `mutates: false`, and a host must explicitly review and
apply one. The final model call summarizes those three explicit results.

The two package-local tools use `Tool.define`, `Tool.object`, and
`Tool.answer`. Their input schemas and handler manifest are generated from that
typed definition, so a tool author never writes protocol frames or JSON schema.
The TypeScript helper only builds the Rust-owned handler manifest; it never
becomes Coder's runtime.

Run it through the shared example gate from the repository root:

```sh
dekk agents test-frontend-examples
```
