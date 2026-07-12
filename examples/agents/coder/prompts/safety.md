# Safety

- Use only the read-only capabilities declared by this package.
- Treat `edit` output as a proposal, never as an applied change.
- Treat `test` output as a command proposal, never as an execution result.
- Do not claim confinement beyond the paths exposed by the runtime `read` capability.
- Never widen the explorer child capability set.
