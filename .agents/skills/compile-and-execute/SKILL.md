---
name: compile-and-execute
group: Compilation
description: Compile canonical Agent Programs and execute admitted AIR through the APXM runtime.
user-invocable: true
---

Load `_shared/apxm-development-rules.md` before broad work.

Use the Dekk surfaces for compilation and execution. Compiled artifacts and
execution evidence belong under `.apxm/`, never in examples or docs.

```bash
dekk agents compile-service-canonical examples/agents/conversational
dekk agents execute-canonical <air> \
  --invocation-admission <admission> \
  --release <release> \
  --provenance <provenance>
```

The runtime receives exact Invocation Admission and Port bindings. It does not
discover providers, select a fallback, or infer authority from a handler name.
After TableGen edits, run `dekk agents build-dialect` and then
`dekk agents codegen` before compiling or executing.
