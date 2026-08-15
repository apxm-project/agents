# Interaction harness example

This package is an ordinary compiled APXM Program. It composes child Programs
through `program.new` and `program.invoke`. The CLI and Runtime Service do not
select those children.

Yield-driven successive input and `await.event` waits remain distinct: the
former commits `CommittedYield`; the latter parks the same Invocation in
`WaitingEvent`.
