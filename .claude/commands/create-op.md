# Create New AIS Operation

Add a new operation end-to-end. Input: **$ARGUMENTS** (e.g., `MyNewOp - does something useful`)

Ask the user for anything not inferrable: category, required/optional fields, latency tier (None/Low/Medium/High), whether it needs a wire index (C++ compiler support).

Read each file before editing. Look at neighboring match arms and existing ops for conventions. Use `NewOp` / `NEW_OP` / `new_op` as placeholders below — substitute the real name.

---

## Step 1: Enum + all match arms

**File**: `crates/core/apxm-ais/src/operations/definitions.rs`

Add **all 8** of these in the file (read it first to find each location):

1. Enum variant in `AISOperationType` (in the right category section)
2. `Display` arm: `AISOperationType::NewOp => write!(f, "NEW_OP"),`
3. `FromStr` arm: `"new_op" => Ok(AISOperationType::NewOp),`
4. `mlir_mnemonic()` arm: `AISOperationType::NewOp => "new_op",`
5. `from_wire_index()` arm — next available index, or skip if Rust-only
6. `to_wire_index()` arm — same index, or `_ => None` if Rust-only
7. `all_operations()` — append to the static array
8. `OperationSpec` entry in `AIS_OPERATIONS` — see existing entries for the template

## Step 2: Constants

**File**: `crates/core/apxm-core/src/constants.rs`

Add attribute keys to `graph::attrs`, belief keys to `runtime::belief_keys`, response keys to `runtime::response_keys` — only for genuinely new strings. Reuse existing constants whenever possible.

## Step 3: Handler

**File**: `crates/runtime/apxm-runtime/src/executor/handlers/<new_op>.rs` (new)

```rust
use super::{ExecutionContext, Node, Result, Value, get_string_attribute};
use apxm_core::constants::graph::attrs as graph_attrs;

pub async fn execute(ctx: &ExecutionContext, node: &Node, inputs: Vec<Value>) -> Result<Value> {
    // TODO: implement
    Ok(Value::Null)
}
```

Copy test boilerplate from an existing handler in the same category (e.g., `nop.rs`, `spawn_agent.rs`).

## Step 4: Register handler module

**File**: `crates/runtime/apxm-runtime/src/executor/handlers/mod.rs`

Add `pub mod <new_op>;` in alphabetical order.

## Step 5: Dispatcher

**File**: `crates/runtime/apxm-runtime/src/executor/dispatcher.rs`

1. Add match arm: `AISOperationType::NewOp => new_op::execute(ctx, node, inputs).await,`
2. Update `all_operations_covered` test count (currently 39)

## Step 6: C++ compiler (skip if Rust-only)

- `crates/compiler/apxm-compiler/mlir/lib/Dialect/AIS/Conversion/Artifact/ArtifactEmitter.cpp` — add `OperationKind` enum value + `.Case<>()` in `mapOperation()`
- `crates/compiler/apxm-compiler/mlir/include/ais/Dialect/AIS/IR/AISOps.td` — add TableGen op def

## Step 7: Docs

Update the AIS operation docs and generated frontend bindings so the new
operation is visible through `dekk apxm ops`.

## Verify

```bash
dekk apxm build
dekk apxm test
```
