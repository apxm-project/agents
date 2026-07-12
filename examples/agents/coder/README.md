# Coder

Coder inspects source with `read`, prepares structured before/after proposals
with `edit`, and prepares test commands for review with `test`. All three
capabilities are read-only: Coder does not apply changes or execute commands.

The package hierarchy permits `explorer` as a child. Explorer receives only
`read`; host orchestration decides whether to invoke it.
