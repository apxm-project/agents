# Task Intake Requirements

Before implementing any feature in `apxm`, provide:

1. A concrete task statement
2. Affected crates/modules
3. Relevant source files
4. Acceptance criteria
5. Required tests

## Invalid examples
- `INVALID TASK`
- vague requests with no behavior specified
- speculative architecture work with no user-facing requirement

## Required repository inspection
Run:

```bash
cd ~/projects/agents/apxm
rg --files -g '*.rs' -g 'Cargo.toml'
```

If the task likely affects routing, inspect:

- `crates/apxm-runtime/src/model_router/`

Do not:
- create parallel routing systems,
- introduce plugin frameworks,
- generalize without a concrete need,
- refactor core dispatch behavior without tests.
