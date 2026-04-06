# Blocked Change Intake Checklist

Use this checklist before attempting any implementation for routing/runtime-related work.

## Stop conditions

If any of the following are true, do not implement:

- task body is missing or invalid,
- repository tree has not been inspected,
- owning crate/module is unknown,
- relevant source files are unavailable,
- current tests are unknown,
- runtime/router invariants are unknown.

## Required intake artifacts

### 1. Real task statement
Must describe the requested behavior change in concrete terms.

### 2. Repository structure
Collect:

```bash
tree -L 3 ~/projects/agents/apxm
```

### 3. Symbol search
Collect:

```bash
rg -n "ContextStack|MemoCache|Scheduler|ModelRouter" ~/projects/agents/apxm
```

### 4. Core manifests and router files
Collect, if present:

```text
~/projects/agents/apxm/Cargo.toml
~/projects/agents/apxm/crates/*/Cargo.toml
~/projects/agents/apxm/crates/apxm-runtime/src/model_router/mod.rs
~/projects/agents/apxm/crates/apxm-runtime/src/model_router/*.rs
```

### 5. Existing tests nearest the target behavior
Identify the tests that already cover the affected path.

## Design guardrails

When implementation becomes possible:

- prefer one authoritative routing path,
- do not create a second source of truth,
- preserve fallback and timeout semantics,
- preserve capability checks and normalization,
- preserve logging/metrics/error conventions,
- keep config changes minimal and unambiguous.

## Deliverables once unblocked

Only after the intake is complete should an implementation plan include:

- exact files to modify,
- exact functions/types to change,
- required manifest changes,
- tests to add/update,
- explicit risk notes.
