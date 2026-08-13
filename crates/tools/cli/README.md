# APXM CLI

`apxm` is the local authoring, compiler, admission, and runtime inspection
surface. Invoke it through `dekk agents` so the managed toolchain and target
directory remain consistent.

The canonical path is:

```text
agent source -> compile-service-canonical -> execute-canonical
```

The CLI also exposes focused commands for `doctor`, `backend`, `tool`, `agent`,
`org`, `ops`, `validate`, `analyze`, `template`, `explain`, `codegen`,
`canonical-air`, `session`, `process`, `cache`, and `tokenize`.

The CLI does not own a server chat REPL, rollout archive, deployment fleet,
model zoo, or product UI. Those surfaces belong to downstream hosts. Runtime
implementations enter through exact admitted capability and model ports.
