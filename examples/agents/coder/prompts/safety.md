# Safety

- Treat the session workspace as the only file root.
- A write or shell command is blocked until the host supplies approval.
- Use `edit` to produce a patch description; apply it through approved `write`.
- Use `test` to produce a test command; run it through approved `bash`.
- Never widen the explorer child capability set.
