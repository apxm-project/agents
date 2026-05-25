---
name: apxm-ais-op-design
description: Use before adding or modifying an AIS op in apxm-core. Enforces the design-before-code gate, the canonical-attribute rule, and the build-dialect + codegen cadence.
user-invocable: true
---

# APXM AIS Op Design

The AIS dialect is the public IR contract — compiler passes, runtime
handlers, the Python frontend, and the vLLM fork all consume it.
**Every other crate consumes; only `apxm-core` defines.**

Load `_shared/apxm-development-rules.md` before broad work.

## When this skill is required

- Adding a new AIS op.
- Renaming, removing, or changing the type signature of an existing op.
- Adding or renaming an op attribute.
- Anything that would change `dekk apxm ops list` output.

## Design-before-code gate

Before touching any `.td` file, answer in writing (in the plan):

1. **Why an op?** What can't be expressed by composing existing ops?
   Three composed ops beat a premature new op.
2. **Op vs. compose decision.** Will every consumer need the
   specialization, or only one pass? If only one — keep it as a
   composition.
3. **Type signature.** What does it consume / produce? What attributes
   does it carry?
4. **Attribute naming.** Use the canonical enum in `apxm-core`. No
   string literals in passes/runtime/Python — see
   `feedback_attribute_dual_naming`.
5. **Frontend impact.** What does the Python frontend need to expose?
   What does `dekk apxm validate` need to accept?
6. **Runtime impact.** Which handler in `crates/runtime/` owns
   dispatch? What does it do with the new op?
7. **vLLM impact.** Does it affect the `/v1/apxm/*` routes? If yes,
   coordinate with `apxm-fork-vllm-rebase`.

Get user sign-off on the design before any code change.

## Implementation cadence

```bash
# 1. Edit the .td definition (and any C++ shim).
$EDITOR crates/core/...

# 2. Rebuild the dialect:
dekk apxm build-dialect

# 3. Regenerate Python frontend bindings:
dekk apxm codegen

# 4. Implement the handler in the runtime:
$EDITOR crates/runtime/...

# 5. Add to the canonical pass list if needed:
$EDITOR crates/compiler/apxm-compiler/src/passes/pipeline.rs

# 6. Targeted tests:
dekk apxm test -p apxm-core
dekk apxm test -p apxm-compiler
dekk apxm test -p apxm-runtime
dekk apxm test-python-frontend

# 7. Surface check:
dekk apxm ops list | grep <new-op>
```

## Rules

- Add only to `apxm-core`. Never define an op outside it.
- Pass list edits go only to `build_pass_list()`.
- Attribute names go through the canonical enum.
- No referential comments in the `.td` ("for plan04", "added by
  task #N"). See `_shared/apxm-agent-operating-rules.md`.

## Anti-patterns

- Adding an op for "future flexibility" with no current consumer.
- Defining an op in `apxm-runtime` because "that's where it's used".
- Skipping `build-dialect`/`codegen` and being confused by phantom
  frontend errors.
- Using a literal string for an attribute name.
- Changing an op's type signature without coordinating with the vLLM
  fork (when the change crosses `/v1/apxm/*`).
