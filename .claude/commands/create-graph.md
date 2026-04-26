# Create Graph Workflow

Build an APXM graph from scratch. Input: **$ARGUMENTS** (describe the workflow goal, e.g., `research assistant that summarizes and critiques`)

---

## Step 1: Discover available operations

```bash
dekk apxm ops list --json
```

Review the operations and identify which ones are needed for the user's workflow goal.

## Step 2: Pick a starter template

```bash
dekk apxm template list
```

If a template matches the workflow pattern, fetch it:

```bash
dekk apxm template show <template-name> > flows/<name>.air
```

Use this as the starting skeleton. If no template fits, start from a minimal AIR module:

```mlir
module {
  func.func @main() -> !ais.token attributes {ais.entry} {
    %answer = ais.ask "Replace this prompt" : !ais.token
    func.return %answer : !ais.token
  }
}
```

## Step 3: Design nodes

For each step in the workflow, create a node with the appropriate AIS operation. Look up op details with:

```bash
dekk apxm ops show <OP_NAME>
```

Each node is an AIS MLIR operation. Use descriptive SSA names and operation-specific attributes.

## Step 4: Design edges

Connect nodes with SSA operands for data flow. Use the operation form documented by `dekk apxm ops show <OP_NAME>`.

## Step 5: Write the graph file

Write the complete AIR source to the user's chosen path (default: `flows/<name>.air`).

## Step 6: Validate

```bash
dekk apxm validate <file>
```

Fix any validation errors before proceeding.

## Step 7: Analyze

```bash
dekk apxm analyze <file>
```

Review parallelism opportunities, critical path, and estimated speedup.

## Step 8: Explain

```bash
dekk apxm explain <file>
```

Confirm the human-readable summary matches the user's intent. Iterate if needed.
