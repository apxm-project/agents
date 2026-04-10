---
name: add-attr
description: Add a new AIS attribute constant and regenerate all downstream projections
user-invocable: false
---

# Add Attribute

How to add a new AIS attribute constant to APXM. Attributes are the canonical field names used across OperationSpec definitions, MLIR TableGen, runtime handlers, and the Python/TypeScript frontends.

## Procedure

### 1. Add the constant

Edit `crates/core/apxm-ais/src/attrs.rs`. Place the constant in the correct domain section:

```rust
// -- Agent / identity --
pub const MY_ATTR: &str = "my_attr";
```

### 2. Add to ALL_ATTR_NAMES

In the same file, add the constant to the `ALL_ATTR_NAMES` array:

```rust
pub const ALL_ATTR_NAMES: &[&str] = &[
    // ...existing entries...
    MY_ATTR,
];
```

### 3. Use in an operation (if applicable)

If the attribute is used by an operation, add it to the operation's `fields` array in `crates/core/apxm-ais/src/operations/definitions.rs`:

```rust
fields: &[
    OperationField::required(attrs::MY_ATTR, "Description of what this field does"),
],
```

### 4. Regenerate codegen

```bash
dekk apxm codegen frontend      # Python _generated/ (constants, operations)
dekk apxm codegen typescript     # TypeScript generated.ts (attrs, ops)
```

### 5. Verify

```bash
# Python constant exists
python -c "from apxm._generated.constants import MY_ATTR; print(MY_ATTR)"

# Consistency test passes (every OperationSpec field references a constant in ALL_ATTR_NAMES)
cargo test -p apxm-ais -- attrs
```

## Key Files

| File | Role |
|------|------|
| `crates/core/apxm-ais/src/attrs.rs` | Constant definition + ALL_ATTR_NAMES |
| `crates/core/apxm-ais/src/operations/definitions.rs` | OperationSpec field references |
| `crates/compiler/apxm-frontend/python/apxm/_generated/constants.py` | Generated Python constants |
| `crates/tools/apxm-gui/frontend/src/types/generated.ts` | Generated TypeScript ATTR object |

## Rules

- Every attribute name MUST be defined as a `pub const` in `attrs.rs`
- Every attribute MUST be added to `ALL_ATTR_NAMES`
- No raw string literals for attribute names anywhere else in the codebase
- The `to_screaming_snake()` conversion in codegen derives the Python/TypeScript name from the constant value
